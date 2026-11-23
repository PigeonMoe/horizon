//! Dedicated-engine SQL ownership checks. A worker owns all database sessions.

use super::{AuthorityCheck, MAX_SNAPSHOT_BYTES, PaperRequest};
use horizon_server_files::pm_sync_store::{ReleasedClaim, Store, StoreError};
use std::{
    sync::{OnceLock, mpsc},
    time::{Duration, Instant},
};

const CHECK_TIMEOUT: Duration = Duration::from_secs(8);

struct Check {
    player: u128,
    handover: u128,
    source: String,
    destination: String,
    token: u128,
    revision: i64,
    digest: [u8; 32],
    expires: Instant,
    reply: mpsc::Sender<Result<(), String>>,
}

pub(super) fn submit_authority(
    request: &PaperRequest<'_>,
    destination: &str,
    token: u128,
) -> Result<AuthorityCheck, String> {
    static WORKER: OnceLock<Option<mpsc::SyncSender<Check>>> = OnceLock::new();
    let revision = i64::try_from(u64::from(request.revision))
        .map_err(|_| "invalid Paper SQL authority revision")?;
    let worker = WORKER
        .get_or_init(|| {
            let (sender, pending) = mpsc::sync_channel::<Check>(1);
            std::thread::Builder::new()
                .name("Paper SQL authority".into())
                .spawn(move || {
                    for check in pending {
                        if Instant::now() < check.expires {
                            let result = check_sql(&check);
                            let _ = check.reply.send(result);
                        }
                    }
                })
                .ok()?;
            Some(sender)
        })
        .as_ref()
        .ok_or("could not start the Paper SQL authority worker")?;
    let (reply, response) = mpsc::channel();
    let deadline = Instant::now() + CHECK_TIMEOUT;
    worker
        .try_send(Check {
            player: request.owner,
            handover: request.handover,
            source: request.source.into(),
            destination: destination.into(),
            token,
            revision,
            digest: request.digest,
            expires: deadline,
            reply,
        })
        .map_err(|_| "the Paper SQL authority worker is busy or unavailable")?;
    Ok(AuthorityCheck { deadline, response })
}

fn check_sql(check: &Check) -> Result<(), String> {
    let url = std::env::var("HORIZON_PM_SYNC_DATABASE_URL")
        .map_err(|_| "the dedicated engine needs HORIZON_PM_SYNC_DATABASE_URL")?;
    let prefix = std::env::var("HORIZON_PM_SYNC_TABLE_PREFIX").unwrap_or_else(|_| "pmsync_".into());
    let mut store = Store::connect_authority(&url, &prefix, MAX_SNAPSHOT_BYTES)
        .map_err(safe_authority_error)?;
    store
        .with_released_claim(
            &ReleasedClaim {
                player: check.player,
                request: check.handover,
                source: &check.source,
                backend: &check.destination,
                token: check.token,
                revision: check.revision,
                snapshot_sha256: check.digest,
            },
            || (),
        )
        .map_err(safe_authority_error)
}

fn safe_authority_error(error: StoreError) -> String {
    match error {
        StoreError::Sql(_) => "the Paper database did not acknowledge its authority check".into(),
        other => other.to_string(),
    }
}
