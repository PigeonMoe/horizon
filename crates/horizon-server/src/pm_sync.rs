//! The local proxy adapter for a departure acknowledged by a Paper PM-Sync backend.

use crate::engine::Control;
use horizon_platform::server_files::{
    pm_sync_spool::{MAX_SNAPSHOT_BYTES, write_checked},
    pm_sync_store::{ReleasedClaim, Store, StoreError},
    properties::parse_uuid,
};
use horizon_server_launch::config::Config;
use serde::Deserialize;
use serde_json::Value;
use std::{io::Read, path::Path};

const MAX_ARGUMENT_BYTES: u64 = 4096;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arrival {
    player: String,
    handover: String,
    source: String,
    owner_token: String,
    revision: i64,
    snapshot_sha256: String,
    compression: Compression,
    prepared: Option<Prepared>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Compression {
    Json,
    Snappy,
}

impl Compression {
    fn name(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Snappy => "snappy",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    record_sha256: String,
    request_sha256: String,
}

impl Arrival {
    fn claim<'a>(&'a self, config: &'a Config) -> Result<ReleasedClaim<'a>, String> {
        let route = &config.transfer;
        let destination = route
            .backend_id
            .as_deref()
            .filter(|_| route.enabled && !route.required_profiles.is_empty())
            .filter(|_| route.allowed_sources.contains(&self.source))
            .ok_or("Paper departure source is outside the configured transfer route")?;
        Ok(ReleasedClaim {
            player: unique_id(&self.player)?,
            request: exact_hex::<16>(&self.handover)
                .map(u128::from_be_bytes)
                .filter(|id| *id != 0)
                .ok_or("invalid PM-Sync handover")?,
            source: &self.source,
            backend: destination,
            token: unique_id(&self.owner_token)?,
            revision: self.revision,
            snapshot_sha256: exact_hex(&self.snapshot_sha256)
                .ok_or("invalid PM-Sync snapshot digest")?,
        })
    }

    fn paper_arguments(&self) -> String {
        format!(
            "{} {} {} {} {} {}",
            self.player,
            self.handover,
            self.source,
            self.revision,
            self.snapshot_sha256,
            self.compression.name()
        )
    }

    fn verify_binding(&self, reply: &Value, destination: &str) -> Result<(), String> {
        let record = reply["record_sha256"].as_str().and_then(exact_hex::<32>);
        let request = reply["request_sha256"].as_str().and_then(exact_hex::<32>);
        if reply["handover"] != self.handover
            || reply["source"] != self.source
            || reply["destination"] != destination
            || reply["authority_revision"] != self.revision
            || reply["paper_snapshot_sha256"] != self.snapshot_sha256
            || record.is_none()
            || request.is_none()
            || self.prepared.as_ref().is_some_and(|expected| {
                reply["record_sha256"] != expected.record_sha256
                    || reply["request_sha256"] != expected.request_sha256
            })
        {
            return Err("the engine reply differs from the exact PM-Sync arrival".into());
        }
        Ok(())
    }
}

fn exact_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let raw = text.as_bytes();
    if raw.len() != 2 * N
        || !raw
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return None;
    }
    let mut out = [0; N];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

fn unique_id(text: &str) -> Result<u128, String> {
    if text.len() == 36
        && text.bytes().enumerate().all(|(index, b)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
        && let Some(id) = parse_uuid(text).filter(|id| *id != 0)
    {
        return Ok(id);
    }
    Err("invalid PM-Sync unique id".into())
}

fn ask(control: &mut Control, line: &str) -> Result<Value, String> {
    let reply = control.ask(line).map_err(|error| error.to_string())?;
    if reply["ok"] == true {
        Ok(reply)
    } else {
        Err(reply["error"]
            .as_str()
            .unwrap_or("the game refused the PM-Sync operation")
            .into())
    }
}

pub fn command(dir: &Path, arguments: &[String]) -> Result<(), String> {
    let [phase] = arguments else {
        return Err("pm-sync-arrival needs prepare, commit or resume".into());
    };
    if !matches!(phase.as_str(), "prepare" | "commit" | "resume") {
        return Err("pm-sync-arrival needs prepare, commit or resume".into());
    }
    let mut input = Vec::new();
    std::io::stdin()
        .lock()
        .take(MAX_ARGUMENT_BYTES + 1)
        .read_to_end(&mut input)
        .map_err(|error| error.to_string())?;
    if input.len() as u64 > MAX_ARGUMENT_BYTES {
        return Err("PM-Sync arrival request is too large".into());
    }
    let arrival: Arrival =
        serde_json::from_slice(&input).map_err(|_| "invalid PM-Sync arrival request")?;
    let prepared = match (phase.as_str(), &arrival.prepared) {
        ("prepare", None) => None,
        ("commit" | "resume", Some(proof))
            if exact_hex::<32>(&proof.record_sha256).is_some()
                && exact_hex::<32>(&proof.request_sha256).is_some() =>
        {
            Some(proof)
        }
        _ => return Err("PM-Sync commit or resume needs both digests returned by prepare".into()),
    };
    let config = Config::load(dir)?;
    let claim = arrival.claim(&config)?;
    let mut control = Control::connect(dir)
        .map_err(|error| format!("could not connect to the game control endpoint: {error}"))?;
    if ask(&mut control, "transfer-status")?["paper_sql_authority"] != true {
        return Err(
            "the dedicated engine needs the PM-Sync MySQL feature and its database settings".into(),
        );
    }
    let paper = arrival.paper_arguments();
    let response = if let Some(proof) = prepared {
        let reply = ask(
            &mut control,
            &format!(
                "player-profile-{phase}-paper {paper} {} {} {}",
                proof.record_sha256, proof.request_sha256, arrival.owner_token,
            ),
        )?;
        arrival.verify_binding(&reply, claim.backend)?;
        let placed = if phase == "resume" {
            "currently_placed"
        } else {
            "placed"
        };
        if reply[placed] != true {
            return Err("the game did not place the authorized Paper arrival".into());
        }
        reply
    } else {
        let url = std::env::var("HORIZON_PM_SYNC_DATABASE_URL")
            .map_err(|_| "HORIZON_PM_SYNC_DATABASE_URL needs the Paper database connection")?;
        let prefix =
            std::env::var("HORIZON_PM_SYNC_TABLE_PREFIX").unwrap_or_else(|_| "pmsync_".into());
        let mut store = Store::connect(&url, &prefix, MAX_SNAPSHOT_BYTES)
            .map_err(|_| "could not connect to the Paper database")?;
        let lease = store
            .claim_released_snapshot(&claim)
            .map_err(|error| match error {
                StoreError::Sql(_) => {
                    "the Paper database did not acknowledge the released claim".into()
                }
                other => other.to_string(),
            })?;
        let bytes = lease
            .snapshot
            .ok_or("the acquired Paper claim has no snapshot")?;
        write_checked(&dir.join(&config.world.name), claim.snapshot_sha256, &bytes)
            .map_err(|error| format!("could not stage the private Paper snapshot: {error}"))?;
        let reply = ask(
            &mut control,
            &format!("player-profile-prepare-paper {paper}"),
        )?;
        arrival.verify_binding(&reply, claim.backend)?;
        if reply["prepared"] != true || reply["placed"] != false {
            return Err("the game did not prepare a held Paper arrival".into());
        }
        reply
    };
    println!("{response}");
    Ok(())
}
