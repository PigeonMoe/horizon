//! The PM-Sync coordinator's durable ownership transitions on the Paper schema.

use std::fmt;

use horizon_protocol::crypto::uuid_string;
use mysql::{Conn, Transaction, TxOpts, prelude::Queryable};
use sha2::{Digest, Sha256};

use crate::{pm_sync_envelope::SnapshotEnvelope, transfer_routes::valid_backend_id};

#[derive(Debug)]
pub enum StoreError {
    Sql(mysql::Error),
    Invalid(&'static str),
    Fenced,
    Conflict,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sql(error) => write!(f, "PM-Sync database operation failed: {error}"),
            Self::Invalid(reason) => f.write_str(reason),
            Self::Fenced => f.write_str("PM-Sync source does not own the expected revision"),
            Self::Conflict => {
                f.write_str("PM-Sync request id already belongs to another operation")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<mysql::Error> for StoreError {
    fn from(error: mysql::Error) -> Self {
        Self::Sql(error)
    }
}

/// The token is private to the coordinator or the backend that holds the lease.
pub struct Lease {
    pub revision: i64,
    pub snapshot: Option<Vec<u8>>,
}

pub struct Handover<'a> {
    pub player: u128,
    pub request: u128,
    pub source: &'a str,
    pub source_token: u128,
    pub source_revision: i64,
    pub destination: &'a str,
    pub destination_token: u128,
    pub snapshot: &'a [u8],
}

/// Only a current receipt can authorize placement. An exact historical receipt
/// reconciles a lost acknowledgement after ownership has moved on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandoverOutcome {
    Committed { revision: i64 },
    CurrentRetry { revision: i64 },
    Historical { revision: i64 },
}

pub struct Store {
    connection: Conn,
    state: String,
    history: String,
    receipts: String,
    max_snapshot_bytes: usize,
}

impl Store {
    /// Open the existing Java schema on a connection supplied by the coordinator.
    /// Keeping all three tables transactional is part of the ownership contract.
    pub fn open(
        mut connection: Conn,
        prefix: &str,
        max_snapshot_bytes: usize,
    ) -> Result<Self, StoreError> {
        if !(1..=40).contains(&prefix.len())
            || !prefix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'_')
            || !(1..=i32::MAX as usize).contains(&max_snapshot_bytes)
        {
            return Err(StoreError::Invalid("invalid PM-Sync database settings"));
        }
        let state = format!("{prefix}state");
        let history = format!("{prefix}history");
        let receipts = format!("{prefix}checkpoint_receipts");
        for table in [&state, &history, &receipts] {
            let engine: Option<String> = connection.exec_first(
                "SELECT ENGINE FROM information_schema.TABLES WHERE TABLE_SCHEMA=DATABASE() AND TABLE_NAME=?",
                (table,),
            )?;
            if !engine.is_some_and(|name| name.eq_ignore_ascii_case("InnoDB")) {
                return Err(StoreError::Invalid(
                    "PM-Sync requires the existing InnoDB tables",
                ));
            }
        }
        Ok(Self {
            connection,
            state,
            history,
            receipts,
            max_snapshot_bytes,
        })
    }

    /// Acquire an unowned profile. The same coordinator retry may recover its
    /// lease using the exact token and backend, including after reconnecting.
    pub fn acquire(
        &mut self,
        player: u128,
        token: u128,
        backend: &str,
    ) -> Result<Lease, StoreError> {
        if player == 0 || token == 0 || !valid_sql_backend(backend) {
            return Err(StoreError::Invalid("invalid PM-Sync lease identity"));
        }
        let player = uuid_string(player);
        let token = uuid_string(token);
        let mut transaction = self.connection.start_transaction(TxOpts::default())?;
        transaction.exec_drop(
            format!("INSERT INTO `{}` (player_uuid) VALUES (?) ON DUPLICATE KEY UPDATE player_uuid=player_uuid", self.state),
            (&player,),
        )?;
        let row: Option<(i64, Option<String>, Option<String>, Option<u64>, Option<Vec<u8>>)> = transaction.exec_first(
            format!("SELECT revision,owner_token,owner_server,OCTET_LENGTH(snapshot),CASE WHEN OCTET_LENGTH(snapshot)<=? THEN snapshot END FROM `{}` WHERE player_uuid=? FOR UPDATE", self.state),
            (self.max_snapshot_bytes, &player),
        )?;
        let (revision, owner, server, length, snapshot) = row.ok_or(StoreError::Fenced)?;
        let unowned = owner.is_none() && server.is_none();
        if !unowned && (owner.as_deref() != Some(&token) || server.as_deref() != Some(backend)) {
            return Err(StoreError::Fenced);
        }
        if revision < 0 || length.is_some_and(|length| length > self.max_snapshot_bytes as u64) {
            return Err(StoreError::Invalid(
                "invalid stored PM-Sync revision or snapshot size",
            ));
        }
        if let Some(bytes) = &snapshot {
            validate_snapshot(bytes, self.max_snapshot_bytes)?;
        }
        if unowned {
            transaction.exec_drop(
                format!("UPDATE `{}` SET owner_token=?,owner_server=?,updated_at=CURRENT_TIMESTAMP WHERE player_uuid=?", self.state),
                (&token, backend, &player),
            )?;
        }
        transaction.commit()?;
        Ok(Lease { revision, snapshot })
    }

    /// Publish both the PM-Sync snapshot and the next owner in one SQL commit.
    /// Calls from the old owner become stale as soon as the row is committed.
    pub fn handover(&mut self, change: &Handover<'_>) -> Result<HandoverOutcome, StoreError> {
        if change.player == 0
            || change.request == 0
            || change.source_token == 0
            || change.destination_token == 0
            || change.source_token == change.destination_token
            || change.source == change.destination
            || !valid_sql_backend(change.source)
            || !valid_sql_backend(change.destination)
        {
            return Err(StoreError::Invalid("invalid PM-Sync hand-over identity"));
        }
        let revision = change
            .source_revision
            .checked_add(1)
            .filter(|revision| *revision > 0)
            .ok_or(StoreError::Invalid("invalid PM-Sync source revision"))?;
        validate_snapshot(change.snapshot, self.max_snapshot_bytes)?;

        let player = uuid_string(change.player);
        let request = uuid_string(change.request);
        let source_token = uuid_string(change.source_token);
        let destination_token = uuid_string(change.destination_token);
        let digest = operation_digest(change);
        let mut transaction = self.connection.start_transaction(TxOpts::default())?;
        let current = lock_head(&mut transaction, &self.state, &player)?;
        let prior: Option<(String, i64)> = transaction.exec_first(
            format!("SELECT payload_digest,revision FROM `{}` WHERE player_uuid=? AND request_id=? FOR UPDATE", self.receipts),
            (&player, &request),
        )?;
        if let Some((prior_digest, prior_revision)) = prior {
            if prior_digest != digest || prior_revision != revision {
                return Err(StoreError::Conflict);
            }
            if current.revision < prior_revision {
                return Err(StoreError::Invalid(
                    "PM-Sync receipt is newer than the authoritative row",
                ));
            }
            let active = current.matches(&destination_token, change.destination, revision);
            transaction.commit()?;
            return Ok(if active {
                HandoverOutcome::CurrentRetry { revision }
            } else {
                HandoverOutcome::Historical { revision }
            });
        }
        if !current.matches(&source_token, change.source, change.source_revision) {
            return Err(StoreError::Fenced);
        }
        transaction.exec_drop(
            format!(
                "INSERT INTO `{}` (player_uuid,revision,snapshot) VALUES (?,?,?)",
                self.history
            ),
            (&player, revision, change.snapshot),
        )?;
        transaction.exec_drop(
            format!("UPDATE `{}` SET snapshot=?,revision=?,owner_token=?,owner_server=?,updated_at=CURRENT_TIMESTAMP WHERE player_uuid=?", self.state),
            (change.snapshot, revision, &destination_token, change.destination, &player),
        )?;
        transaction.exec_drop(
            format!(
                "DELETE FROM `{}` WHERE player_uuid=? AND revision<=?",
                self.history
            ),
            (&player, revision - 16),
        )?;
        transaction.exec_drop(
            format!("INSERT INTO `{}` (player_uuid,request_id,payload_digest,revision) VALUES (?,?,?,?)", self.receipts),
            (&player, &request, &digest, revision),
        )?;
        transaction.commit()?;
        Ok(HandoverOutcome::Committed { revision })
    }
}

fn valid_sql_backend(id: &str) -> bool {
    id.len() <= 64 && valid_backend_id(id)
}

fn validate_snapshot(bytes: &[u8], max_snapshot_bytes: usize) -> Result<(), StoreError> {
    SnapshotEnvelope::decode(bytes, max_snapshot_bytes)
        .map(|_| ())
        .map_err(|_| StoreError::Invalid("invalid PM-Sync v2 snapshot"))
}

struct Head {
    revision: i64,
    token: Option<String>,
    backend: Option<String>,
}

impl Head {
    fn matches(&self, token: &str, backend: &str, revision: i64) -> bool {
        self.revision == revision
            && self.token.as_deref() == Some(token)
            && self.backend.as_deref() == Some(backend)
    }
}

fn lock_head(
    transaction: &mut Transaction<'_>,
    table: &str,
    player: &str,
) -> Result<Head, StoreError> {
    let row: Option<(i64, Option<String>, Option<String>)> = transaction.exec_first(
        format!(
            "SELECT revision,owner_token,owner_server FROM `{table}` WHERE player_uuid=? FOR UPDATE"
        ),
        (player,),
    )?;
    let (revision, token, backend) = row.ok_or(StoreError::Fenced)?;
    Ok(Head {
        revision,
        token,
        backend,
    })
}

fn operation_digest(change: &Handover<'_>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"horizon/pm-sync/ownership/v1\0");
    for uuid in [
        change.player,
        change.request,
        change.source_token,
        change.destination_token,
    ] {
        digest.update(uuid.to_be_bytes());
    }
    digest.update(change.source_revision.to_be_bytes());
    for id in [change.source, change.destination] {
        digest.update([id.len() as u8]);
        digest.update(id.as_bytes());
    }
    digest.update(Sha256::digest(change.snapshot));
    format!("{:x}", digest.finalize())
}
