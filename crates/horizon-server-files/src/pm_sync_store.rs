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
    DuplicateCheckpoint,
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
            Self::DuplicateCheckpoint => {
                f.write_str("PM-Sync checkpoint already committed; query its receipt")
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointDisposition {
    KeepLease,
    ReleaseLease,
}

pub struct Checkpoint<'a> {
    pub player: u128,
    pub request: u128,
    pub backend: &'a str,
    pub token: u128,
    pub revision: i64,
    pub disposition: CheckpointDisposition,
    pub snapshot: &'a [u8],
}

/// Exact, externally authorized Paper departure. The SQL row can verify the
/// released bytes and revision. The coordinator supplies the source identity
/// from the current Paper connection's authenticated departure notice.
pub struct ReleasedClaim<'a> {
    pub player: u128,
    pub request: u128,
    pub source: &'a str,
    pub backend: &'a str,
    pub token: u128,
    pub revision: i64,
    pub snapshot_sha256: [u8; 32],
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
        let (current, snapshot) = lock_snapshot(
            &mut transaction,
            &self.state,
            &player,
            self.max_snapshot_bytes,
        )?;
        let unowned = current.is_unowned();
        if !unowned && !current.matches(&token, backend, current.revision) {
            return Err(StoreError::Fenced);
        }
        if current.revision < 0 {
            return Err(StoreError::Invalid("invalid stored PM-Sync revision"));
        }
        if let Some(bytes) = &snapshot {
            validate_snapshot(bytes, self.max_snapshot_bytes)?;
        }
        if unowned {
            set_owner(&mut transaction, &self.state, &player, &token, backend)?;
        }
        transaction.commit()?;
        Ok(Lease {
            revision: current.revision,
            snapshot,
        })
    }

    /// Claim one exact released Paper snapshot. This operation cannot create an
    /// empty profile or take a profile from a different backend or revision.
    /// Lost responses can be retried using the same request and target token.
    pub fn claim_released_snapshot(
        &mut self,
        claim: &ReleasedClaim<'_>,
    ) -> Result<Lease, StoreError> {
        if claim.player == 0
            || claim.request == 0
            || claim.token == 0
            || claim.revision < 1
            || claim.source == claim.backend
            || !valid_sql_backend(claim.source)
            || !valid_sql_backend(claim.backend)
        {
            return Err(StoreError::Invalid(
                "invalid PM-Sync released snapshot claim",
            ));
        }
        let digest = claim_digest(claim);
        let (player, request, token) = (
            uuid_string(claim.player),
            uuid_string(claim.request),
            uuid_string(claim.token),
        );
        let mut transaction = self.connection.start_transaction(TxOpts::default())?;
        let (current, snapshot) = lock_snapshot(
            &mut transaction,
            &self.state,
            &player,
            self.max_snapshot_bytes,
        )?;
        let retried = matches_receipt(
            &mut transaction,
            &self.receipts,
            &player,
            &request,
            &digest,
            claim.revision,
        )?;
        if current.revision != claim.revision
            || (retried && !current.matches(&token, claim.backend, claim.revision))
            || (!retried && !current.is_unowned())
        {
            return Err(StoreError::Fenced);
        }
        let snapshot = snapshot.ok_or(StoreError::Fenced)?;
        let sha256: [u8; 32] = Sha256::digest(&snapshot).into();
        if sha256 != claim.snapshot_sha256 {
            return Err(StoreError::Fenced);
        }
        validate_snapshot(&snapshot, self.max_snapshot_bytes)?;
        if !retried {
            set_owner(
                &mut transaction,
                &self.state,
                &player,
                &token,
                claim.backend,
            )?;
            write_receipt(
                &mut transaction,
                &self.receipts,
                &player,
                &request,
                &digest,
                claim.revision,
            )?;
        }
        transaction.commit()?;
        Ok(Lease {
            revision: claim.revision,
            snapshot: Some(snapshot),
        })
    }

    /// Read the revision of an exact live lease without acquiring one. This
    /// also works after that same owner has committed more checkpoints.
    pub fn current_owner_revision(
        &mut self,
        player: u128,
        token: u128,
        backend: &str,
    ) -> Result<Option<i64>, StoreError> {
        if player == 0 || token == 0 || !valid_sql_backend(backend) {
            return Err(StoreError::Invalid("invalid PM-Sync lease identity"));
        }
        let row: Option<(i64, Option<String>, Option<String>)> = self.connection.exec_first(
            format!(
                "SELECT revision,owner_token,owner_server FROM `{}` WHERE player_uuid=?",
                self.state
            ),
            (uuid_string(player),),
        )?;
        let token = uuid_string(token);
        Ok(row.and_then(|(revision, owned, server)| {
            (revision >= 0
                && owned.as_deref() == Some(&token)
                && server.as_deref() == Some(backend))
            .then_some(revision)
        }))
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
        let revision = next_revision(change.source_revision)?;
        validate_snapshot(change.snapshot, self.max_snapshot_bytes)?;

        let player = uuid_string(change.player);
        let request = uuid_string(change.request);
        let source_token = uuid_string(change.source_token);
        let destination_token = uuid_string(change.destination_token);
        let digest = operation_digest(change);
        let mut transaction = self.connection.start_transaction(TxOpts::default())?;
        let current = lock_head(&mut transaction, &self.state, &player)?;
        if matches_receipt(
            &mut transaction,
            &self.receipts,
            &player,
            &request,
            &digest,
            revision,
        )? {
            if current.revision < revision {
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
        write_snapshot(
            &mut transaction,
            (&self.state, &self.history),
            &player,
            change.snapshot,
            &Head {
                revision,
                token: Some(destination_token),
                backend: Some(change.destination.to_owned()),
            },
        )?;
        write_receipt(
            &mut transaction,
            &self.receipts,
            &player,
            &request,
            &digest,
            revision,
        )?;
        transaction.commit()?;
        Ok(HandoverOutcome::Committed { revision })
    }

    /// Commit a live checkpoint or release the lease with the last saved state.
    /// The original Paper plugin can acquire the released snapshot on its next join.
    pub fn checkpoint(&mut self, change: &Checkpoint<'_>) -> Result<i64, StoreError> {
        let (revision, digest) = self.checkpoint_binding(change)?;
        let (player, request, token) = (
            uuid_string(change.player),
            uuid_string(change.request),
            uuid_string(change.token),
        );
        let mut transaction = self.connection.start_transaction(TxOpts::default())?;
        let current = lock_head(&mut transaction, &self.state, &player)?;
        if matches_receipt(
            &mut transaction,
            &self.receipts,
            &player,
            &request,
            &digest,
            revision,
        )? {
            return Err(StoreError::DuplicateCheckpoint);
        }
        if !current.matches(&token, change.backend, change.revision) {
            return Err(StoreError::Fenced);
        }
        let keep = change.disposition == CheckpointDisposition::KeepLease;
        write_snapshot(
            &mut transaction,
            (&self.state, &self.history),
            &player,
            change.snapshot,
            &Head {
                revision,
                token: keep.then_some(token),
                backend: keep.then(|| change.backend.to_owned()),
            },
        )?;
        write_receipt(
            &mut transaction,
            &self.receipts,
            &player,
            &request,
            &digest,
            revision,
        )?;
        transaction.commit()?;
        Ok(revision)
    }

    /// Reconcile the exact checkpoint after an ambiguous response. A receipt
    /// acknowledges the write, even after another backend owns a newer revision.
    pub fn checkpoint_receipt(
        &mut self,
        change: &Checkpoint<'_>,
    ) -> Result<Option<i64>, StoreError> {
        let (revision, digest) = self.checkpoint_binding(change)?;
        let (player, request) = (uuid_string(change.player), uuid_string(change.request));
        let prior = self.connection.exec_first(
            format!(
                "SELECT payload_digest,revision FROM `{}` WHERE player_uuid=? AND request_id=?",
                self.receipts
            ),
            (&player, &request),
        )?;
        Ok(check_receipt(prior, &digest, revision)?.then_some(revision))
    }

    fn checkpoint_binding(&self, change: &Checkpoint<'_>) -> Result<(i64, String), StoreError> {
        if change.player == 0
            || change.request == 0
            || change.token == 0
            || !valid_sql_backend(change.backend)
        {
            return Err(StoreError::Invalid("invalid PM-Sync checkpoint identity"));
        }
        let revision = next_revision(change.revision)?;
        validate_snapshot(change.snapshot, self.max_snapshot_bytes)?;
        let mut digest = Sha256::new();
        digest.update(b"horizon/pm-sync/checkpoint/v1\0");
        for uuid in [change.player, change.request, change.token] {
            digest.update(uuid.to_be_bytes());
        }
        digest.update(change.revision.to_be_bytes());
        digest.update([u8::from(
            change.disposition == CheckpointDisposition::ReleaseLease,
        )]);
        digest.update([change.backend.len() as u8]);
        digest.update(change.backend.as_bytes());
        digest.update(Sha256::digest(change.snapshot));
        Ok((revision, format!("{:x}", digest.finalize())))
    }
}

fn next_revision(revision: i64) -> Result<i64, StoreError> {
    revision
        .checked_add(1)
        .filter(|revision| *revision > 0)
        .ok_or(StoreError::Invalid("invalid PM-Sync source revision"))
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
    fn is_unowned(&self) -> bool {
        self.token.is_none() && self.backend.is_none()
    }

    fn matches(&self, token: &str, backend: &str, revision: i64) -> bool {
        self.revision == revision
            && self.token.as_deref() == Some(token)
            && self.backend.as_deref() == Some(backend)
    }
}

fn lock_snapshot(
    transaction: &mut Transaction<'_>,
    table: &str,
    player: &str,
    max_bytes: usize,
) -> Result<(Head, Option<Vec<u8>>), StoreError> {
    let row: Option<(i64, Option<String>, Option<String>, Option<u64>, Option<Vec<u8>>)> =
        transaction.exec_first(
            format!("SELECT revision,owner_token,owner_server,OCTET_LENGTH(snapshot),CASE WHEN OCTET_LENGTH(snapshot)<=? THEN snapshot END FROM `{table}` WHERE player_uuid=? FOR UPDATE"),
            (max_bytes, player),
        )?;
    let (revision, token, backend, length, snapshot) = row.ok_or(StoreError::Fenced)?;
    if length.is_some_and(|length| length > max_bytes as u64) {
        return Err(StoreError::Invalid(
            "stored PM-Sync snapshot exceeds its size limit",
        ));
    }
    Ok((
        Head {
            revision,
            token,
            backend,
        },
        snapshot,
    ))
}

fn set_owner(
    transaction: &mut Transaction<'_>,
    table: &str,
    player: &str,
    token: &str,
    backend: &str,
) -> Result<(), StoreError> {
    transaction.exec_drop(
        format!("UPDATE `{table}` SET owner_token=?,owner_server=?,updated_at=CURRENT_TIMESTAMP WHERE player_uuid=?"),
        (token, backend, player),
    )?;
    Ok(())
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

fn check_receipt(
    prior: Option<(String, i64)>,
    digest: &str,
    revision: i64,
) -> Result<bool, StoreError> {
    match prior {
        Some((bound, prior_revision)) if bound == digest && prior_revision == revision => Ok(true),
        Some(_) => Err(StoreError::Conflict),
        None => Ok(false),
    }
}

fn matches_receipt(
    transaction: &mut Transaction<'_>,
    table: &str,
    player: &str,
    request: &str,
    digest: &str,
    revision: i64,
) -> Result<bool, StoreError> {
    let prior = transaction.exec_first(
        format!("SELECT payload_digest,revision FROM `{table}` WHERE player_uuid=? AND request_id=? FOR UPDATE"),
        (player, request),
    )?;
    check_receipt(prior, digest, revision)
}

fn write_snapshot(
    transaction: &mut Transaction<'_>,
    (state, history): (&str, &str),
    player: &str,
    snapshot: &[u8],
    next: &Head,
) -> Result<(), StoreError> {
    transaction.exec_drop(
        format!("INSERT INTO `{history}` (player_uuid,revision,snapshot) VALUES (?,?,?)"),
        (player, next.revision, snapshot),
    )?;
    transaction.exec_drop(
        format!("UPDATE `{state}` SET snapshot=?,revision=?,owner_token=?,owner_server=?,updated_at=CURRENT_TIMESTAMP WHERE player_uuid=?"),
        (snapshot, next.revision, next.token.as_deref(), next.backend.as_deref(), player),
    )?;
    transaction.exec_drop(
        format!("DELETE FROM `{history}` WHERE player_uuid=? AND revision<=?"),
        (player, next.revision - 16),
    )?;
    Ok(())
}

fn write_receipt(
    transaction: &mut Transaction<'_>,
    table: &str,
    player: &str,
    request: &str,
    digest: &str,
    revision: i64,
) -> Result<(), StoreError> {
    transaction.exec_drop(
        format!("INSERT INTO `{table}` (player_uuid,request_id,payload_digest,revision) VALUES (?,?,?,?)"),
        (player, request, digest, revision),
    )?;
    Ok(())
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

fn claim_digest(claim: &ReleasedClaim<'_>) -> String {
    let mut digest = Sha256::new();
    digest.update(b"horizon/pm-sync/released-claim/v1\0");
    for uuid in [claim.player, claim.request, claim.token] {
        digest.update(uuid.to_be_bytes());
    }
    digest.update(claim.revision.to_be_bytes());
    for id in [claim.source, claim.backend] {
        digest.update([id.len() as u8]);
        digest.update(id.as_bytes());
    }
    digest.update(claim.snapshot_sha256);
    format!("{:x}", digest.finalize())
}
