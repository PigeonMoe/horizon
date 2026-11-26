//! Prepare one native player-import request from a Paper PM-Sync snapshot
//! deposited by the coordinator. Preparation never publishes a player file.

use super::receipt::{self, Binding, digest_hex, parse_authority_revision, parse_digest};
use crate::game::{
    Game,
    stats::{Stats, stat_registry},
};
use chrono::{DateTime, Utc};
use horizon_client::{
    advancements::{Progress, Tree},
    lan::{
        arrivals, base64,
        hub::Hub,
        playerdata,
        pm_sync_paper::{PaperMigrationContext, stage_paper_migration},
        source_handover::AuthorityRevision,
    },
};
use horizon_core::{
    identifier::Identifier,
    nbt::{Limits, NamedTag, Tag, write_named_strict},
};
use horizon_items::husksync_packed::{PackedCompression, PackedLimits};
use horizon_protocol::crypto::uuid_string;
use horizon_server_files::{
    pm_sync_envelope::SnapshotEnvelope,
    pm_sync_spool::{self, MAX_SNAPSHOT_BYTES},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::mpsc, time::Instant};

#[cfg(feature = "pm-sync-mysql")]
#[path = "transfer_paper_authority.rs"]
mod authority;

struct PaperRequest<'a> {
    owner: u128,
    handover: u128,
    source: &'a str,
    revision: AuthorityRevision,
    digest: [u8; 32],
    compression: PackedCompression,
}

impl<'a> PaperRequest<'a> {
    fn parse(args: &'a str) -> Result<Self, String> {
        let fields = args.split_whitespace().take(7).collect::<Vec<_>>();
        let [owner, handover, source, revision, digest, compression] = fields.as_slice() else {
            return Err("Paper profile preparation needs a unique id, hand-over, source, revision, snapshot digest and compression".into());
        };
        let owner = arrivals::parse_uuid(owner)
            .filter(|id| *id != 0)
            .ok_or("invalid player unique id")?;
        let handover = arrivals::parse_handover(handover)
            .filter(|id| *id != 0)
            .ok_or("invalid hand-over id")?;
        let compression = match *compression {
            "json" => PackedCompression::Json,
            "snappy" => PackedCompression::Snappy,
            _ => return Err("Paper snapshot compression must be json or snappy".into()),
        };
        Ok(Self {
            owner,
            handover,
            source,
            revision: parse_authority_revision(revision)?,
            digest: parse_digest(digest).ok_or("invalid Paper snapshot digest")?,
            compression,
        })
    }
}

struct PaperImport<'a> {
    paper: PaperRequest<'a>,
    record: &'a str,
    request: &'a str,
}

impl<'a> PaperImport<'a> {
    fn parse(args: &'a str) -> Result<Self, String> {
        let (before, request) = args
            .trim_end()
            .rsplit_once(char::is_whitespace)
            .ok_or("Paper profile import needs a prepared record and request digest")?;
        let (paper, record) = before
            .trim_end()
            .rsplit_once(char::is_whitespace)
            .ok_or("Paper profile import needs a prepared record and request digest")?;
        Ok(Self {
            paper: PaperRequest::parse(paper)?,
            record,
            request,
        })
    }
}

#[derive(Clone, Copy)]
enum Activation {
    Commit,
    Resume,
}

impl Activation {
    fn verb(self) -> &'static str {
        match self {
            Self::Commit => "player-profile-commit-paper",
            Self::Resume => "player-profile-resume-paper",
        }
    }
}

struct AuthorityCheck {
    deadline: Instant,
    response: mpsc::Receiver<Result<(), String>>,
}

impl AuthorityCheck {
    fn poll(&self, now: Instant) -> Option<Result<(), String>> {
        if now >= self.deadline {
            return Some(Err("the Paper SQL authority check timed out".into()));
        }
        match self.response.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err("the Paper SQL authority worker did not answer".into()))
            }
        }
    }
}

struct PaperIntent {
    action: Activation,
    import: String,
    binding: Binding,
    connection: Option<u32>,
}

pub(in crate::game::dedicated) struct PendingPaperActivation {
    intent: PaperIntent,
    check: AuthorityCheck,
    reply: mpsc::Sender<String>,
}

#[cfg(not(feature = "pm-sync-mysql"))]
fn submit_authority(_: &PaperRequest<'_>, _: &str, _: u128) -> Result<AuthorityCheck, String> {
    Err("this dedicated engine was built without PM-Sync MySQL authority".into())
}

#[cfg(feature = "pm-sync-mysql")]
use authority::submit_authority;

impl Game<'_> {
    pub(in crate::game::dedicated) fn start_paper_request(
        &self,
        verb: &str,
        args: &str,
        reply: mpsc::Sender<String>,
        pending: &mut Option<PendingPaperActivation>,
    ) {
        let action = match verb {
            "player-profile-commit-paper" => Activation::Commit,
            _ => Activation::Resume,
        };
        if pending.is_some() {
            send_paper_reply(
                action,
                reply,
                Err("a Paper SQL authority check is already pending".into()),
            );
            return;
        }
        match self.submit_paper_activation(args, action, submit_authority) {
            Ok((intent, check)) => {
                *pending = Some(PendingPaperActivation {
                    intent,
                    check,
                    reply,
                })
            }
            Err(error) => send_paper_reply(action, reply, Err(error)),
        }
    }

    fn submit_paper_activation(
        &self,
        args: &str,
        action: Activation,
        submit: impl FnOnce(&PaperRequest<'_>, &str, u128) -> Result<AuthorityCheck, String>,
    ) -> Result<(PaperIntent, AuthorityCheck), String> {
        let (import_args, token) = args
            .trim_end()
            .rsplit_once(char::is_whitespace)
            .ok_or("Paper activation needs a prepared arrival and SQL owner token")?;
        let import = PaperImport::parse(import_args)?;
        let token = arrivals::parse_uuid(token)
            .filter(|token| *token != 0)
            .ok_or("invalid Paper SQL owner token")?;
        let (hub, settings, destination) = self.paper_activation_route()?;
        let binding = paper_binding(settings, &import, destination)?;
        let connection = paper_connection(hub, &binding);
        if connection.is_none() && matches!(action, Activation::Resume) {
            return Err("no expected forwarded Paper player is connected".into());
        }
        let check = submit(&import.paper, destination, token)?;
        Ok((
            PaperIntent {
                action,
                import: import_args.into(),
                binding,
                connection,
            },
            check,
        ))
    }

    pub(in crate::game::dedicated) fn poll_paper_request(
        &mut self,
        pending: &mut Option<PendingPaperActivation>,
    ) {
        let Some(activation) = pending.as_ref() else {
            return;
        };
        let result = match self.paper_activation_route() {
            Err(error) => Some(Err(error)),
            Ok(_) => activation.check.poll(Instant::now()),
        };
        let Some(result) = result else { return };
        let activation = pending
            .take()
            .expect("the pending activation was checked above");
        let reply = result.and_then(|()| self.complete_paper_activation(&activation.intent));
        send_paper_reply(activation.intent.action, activation.reply, reply);
    }

    fn paper_activation_route(&self) -> Result<(&Hub, &arrivals::Settings, &str), String> {
        let hub = &self
            .lan
            .as_ref()
            .ok_or("the server is not open to players")?
            .hub;
        let transfer = hub.transfers.as_ref().ok_or(arrivals::OFF)?;
        if transfer.stopping {
            return Err(arrivals::STOPPING.into());
        }
        let net = &hub.lan.shared.config;
        if transfer.settings.required_profiles.is_empty()
            || net.forwarding_secret.is_none()
            || net.online.is_some()
        {
            return Err(
                "Paper profile activation needs required profiles and authenticated forwarding"
                    .into(),
            );
        }
        let destination = transfer
            .settings
            .backend_id
            .as_deref()
            .ok_or("the destination is not configured for strict transfer")?;
        Ok((hub, &transfer.settings, destination))
    }

    fn complete_paper_activation(&mut self, intent: &PaperIntent) -> Result<Value, String> {
        let (hub, settings, destination) = self.paper_activation_route()?;
        let import = PaperImport::parse(&intent.import)?;
        let binding = paper_binding(settings, &import, destination)?;
        if binding != intent.binding {
            return Err(
                "the Paper profile route or roster changed while checking authority".into(),
            );
        }
        // A request without a connected player can acknowledge history, even if a new player
        // arrived while SQL was running. That request must never attach to the new connection.
        let Some(connection) = intent.connection else {
            let committed = receipt::lookup(&self.world_root, &binding)?
                .filter(|entry| entry.paper_snapshot_sha256 == Some(import.paper.digest))
                .ok_or("no expected forwarded Paper player is connected")?;
            return Ok(committed.commit_reply(false, true));
        };
        if paper_connection(hub, &binding) != Some(connection) {
            return Err("the forwarded Paper connection changed while checking authority".into());
        }
        if hub.source_frozen_handover(binding.owner).is_some()
            && hub
                .held_profile_return(binding.owner)
                .is_none_or(|returning| !binding.matches_return(returning))
        {
            return Err("a source hand-over still fences this Paper player".into());
        }
        let destination = destination.to_owned();
        match intent.action {
            Activation::Commit => {
                let staged = self.stage_exact_paper_profile(&import, &binding)?;
                if staged["ok"] != true {
                    return Ok(staged);
                }
                Ok(self.confirm_bound_profile_arrival(&binding, Some(import.paper.digest)))
            }
            Activation::Resume => {
                self.resume_exact_paper_profile(&binding, import.paper.digest, &destination)
            }
        }
    }

    fn resume_exact_paper_profile(
        &mut self,
        binding: &Binding,
        paper_digest: [u8; 32],
        destination: &str,
    ) -> Result<Value, String> {
        let hub = &self.lan.as_ref().expect("the server is open").hub;
        let (owner, id) = (binding.owner, binding.handover);
        let mut returning = hub.held_profile_return(owner).cloned();
        if returning.is_none()
            && hub.placed_arrival(owner, id).is_some()
            && let Some(journal) = &hub.lan.shared.config.source_handovers
        {
            returning = journal
                .current_return(destination, owner)
                .map_err(|error| error.to_string())?
                .map(|current| current.authorization().clone());
        }
        if let Some(returning) = returning {
            let committed = receipt::lookup(&self.world_root, binding)?
                .filter(|entry| entry.paper_snapshot_sha256 == Some(paper_digest))
                .filter(|_| binding.matches_return(&returning))
                .ok_or("the current return differs from the Paper arrival receipt")?;
            let live = self.resume_committed_profile(&returning, Some(paper_digest))?;
            return Ok(paper_resume_reply(&committed, live));
        }
        if hub.source_frozen_handover(owner).is_some() {
            return Err("a source hand-over still fences this Paper player".into());
        }
        let guest = hub
            .guests
            .iter()
            .find(|guest| {
                guest.uuid == owner
                    && guest.expected
                    && guest.participates_online()
                    && guest.left_reason.is_empty()
            })
            .ok_or("no expected forwarded Paper player is connected")?;
        let live = guest.placed && guest.arrival_handover == Some(id);
        if !live
            && (!guest.held
                || guest.placed
                || guest.arrival_handover.is_some_and(|staged| staged != id))
        {
            return Err("the forwarded connection belongs to a different arrival".into());
        }
        self.finish_player_autosaves(true)
            .map_err(|error| error.to_string())?;
        let current = receipt::CurrentPaperArrival::read(&self.world_root, binding, paper_digest)?;
        self.verify_committed_profiles(owner, id, &current.player)?;
        if !live {
            self.restore_held_profile(owner, id, current.player)?;
            self.finish_arrival(owner, id, None, false)?;
        }
        Ok(paper_resume_reply(&current.receipt, live))
    }

    pub(super) fn prepare_paper_profile(&mut self, args: &str) -> Value {
        let result =
            PaperRequest::parse(args).and_then(|request| self.project_paper_profile(&request));
        match result {
            Ok(mut prepared) => {
                // The coordinator receives the tuple. The larger native record
                // stays inside the engine and is rebuilt from the verified spool.
                prepared
                    .as_object_mut()
                    .expect("prepared reply is an object")
                    .remove("request");
                prepared
            }
            Err(error) => json!({ "error": error }),
        }
    }

    pub(super) fn stage_paper_profile(&mut self, args: &str) -> Value {
        let result = (|| {
            let import = PaperImport::parse(args)?;
            let host = self
                .lan
                .as_ref()
                .ok_or("the server is not open to players")?;
            let transfer = host.hub.transfers.as_ref().ok_or(arrivals::OFF)?;
            let destination = transfer
                .settings
                .backend_id
                .as_deref()
                .ok_or("the destination is not configured for strict transfer")?;
            let binding = paper_binding(&transfer.settings, &import, destination)?;
            self.stage_exact_paper_profile(&import, &binding)
        })();
        result.unwrap_or_else(|error| json!({ "error": error }))
    }

    fn stage_exact_paper_profile(
        &mut self,
        import: &PaperImport<'_>,
        binding: &Binding,
    ) -> Result<Value, String> {
        let request = &import.paper;
        let host = self.lan.as_ref().expect("the server is open");
        if let Some(committed) = receipt::lookup(&self.world_root, binding)? {
            if committed.paper_snapshot_sha256 != Some(request.digest) {
                return Err("the Paper snapshot differs from its durable arrival receipt".into());
            }
            let placed = host
                .hub
                .placed_arrival(request.owner, request.handover)
                .is_some();
            return Ok(committed.commit_reply(placed, true));
        }
        if let Ok(index) = host.hub.imported_arrival(request.owner, request.handover) {
            let staged = receipt::staged(&host.hub.guests[index])?
                .ok_or("the held Paper profile has no staged receipt")?;
            if staged.binding != *binding || staged.paper_snapshot_sha256 != Some(request.digest) {
                return Err("the Paper snapshot differs from its held profile stage".into());
            }
            let mut reply = json!({"ok": true, "placed": false, "retried": true,
                "paper_snapshot_sha256": digest_hex(request.digest) });
            binding.add_to_reply(&mut reply);
            return Ok(reply);
        }
        let mut projected = self.project_paper_profile(request)?;
        if projected["record_sha256"].as_str() != Some(import.record)
            || projected["request_sha256"].as_str() != Some(import.request)
        {
            return Err("the Paper projection changed since it was prepared".into());
        }
        Ok(self.apply_player_import(request.owner, projected["request"].take(), true))
    }

    fn project_paper_profile(&mut self, request: &PaperRequest<'_>) -> Result<Value, String> {
        let tree = self.advancement_tree();
        let host = self
            .lan
            .as_ref()
            .ok_or("the server is not open to players")?;
        let transfer = host.hub.transfers.as_ref().ok_or(arrivals::OFF)?;
        if transfer.stopping {
            return Err(arrivals::STOPPING.into());
        }
        if host.hub.lan.shared.config.forwarding_secret.is_none() {
            return Err("Paper profile preparation needs authenticated forwarding".into());
        }
        let settings = &transfer.settings;
        let destination = settings
            .backend_id
            .as_deref()
            .filter(|destination| *destination != request.source)
            .filter(|_| settings.allowed_sources.contains(request.source))
            .ok_or("the Paper source is not approved for this destination")?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|guest| {
                guest.uuid == request.owner
                    && guest.held
                    && guest.expected
                    && !guest.placed
                    && !guest.imported
                    && guest.participates_online()
                    && guest.left_reason.is_empty()
            })
            .ok_or("no expected Paper player with that unique id is held")?;

        let raw = pm_sync_spool::read(&self.world_root, request.digest)
            .map_err(|error| format!("could not read the staged Paper snapshot: {error}"))?;
        let envelope = SnapshotEnvelope::decode(&raw, MAX_SNAPSHOT_BYTES)
            .map_err(|error| error.to_string())?;
        let Tag::Compound(player) = playerdata::player_tag(guest) else {
            return Err("the held player's native record is invalid".into());
        };
        let mut projected = stage_paper_migration(
            &envelope,
            &player,
            &PaperMigrationContext {
                items: self.res.items(),
                attributes: &self.motion.attributes,
                recipes: self.display_index(),
                compression: request.compression,
                limits: PackedLimits {
                    max_payload_bytes: MAX_SNAPSHOT_BYTES,
                    max_json_bytes: MAX_SNAPSHOT_BYTES,
                },
            },
        )
        .map_err(|error| error.to_string())?;
        // Keep the portable record free of this world's generation and old receipt metadata.
        for key in [
            "HorizonTransferProfiles",
            "HorizonTransferAuthority",
            "HorizonArrivalReceipts",
            "HorizonStagedArrivalReceipt",
            receipt::PAPER_AUTHORITY_KEY,
        ] {
            projected.player.remove(&key.into());
        }
        let encoded = write_named_strict(
            &NamedTag {
                name: Default::default(),
                tag: Tag::Compound(projected.player),
            },
            Limits::saved(),
        )
        .map_err(|error| error.to_string())?;
        let mut stats = Stats::default();
        if let Some(play) = host.play.get(&guest.conn) {
            stats.values.clone_from(&play.stats.values);
        }
        let known = |registry: &str, key: &str| {
            let (Some(registry), Some(key)) = (
                Identifier::parse_namespaced(registry),
                Identifier::parse_namespaced(key),
            ) else {
                return false;
            };
            host.hub.synced_id(&registry, &key).is_some()
        };
        projected
            .statistics
            .overlay_native(&mut stats.values, |kind, key| {
                known("minecraft:stat_type", kind)
                    && stat_registry(kind).is_some_and(|registry| known(registry, key))
            });
        let advancements =
            project_advancements(&tree, &guest.advancements, &projected.advancements)?
                .to_json(&tree, horizon_world::DATA_VERSION);
        let profiles = requested_paper_sections(&settings.required_profiles, &projected.sections)?;
        let native = json!({
            "record": {
                "version": 2,
                "data_version": horizon_world::DATA_VERSION,
                "uuid": uuid_string(request.owner),
                "handover": arrivals::handover_text(request.handover),
                "source": request.source,
                "destination": destination,
                "authority_revision": u64::from(request.revision),
                "paper_snapshot_sha256": digest_hex(request.digest),
                "player": base64(&encoded),
                "advancements": advancements,
                "stats": stats.to_json(),
                "profiles": profiles,
            },
            "place": false,
        });
        let binding = Binding::from_import(settings, request.owner, &native)?;
        if host
            .hub
            .held_profile_return(request.owner)
            .is_some_and(|authorized| !binding.matches_return(authorized))
        {
            return Err(
                "the prepared Paper record differs from the held return authorization".into(),
            );
        }
        let mut reply = json!({
            "ok": true,
            "prepared": true,
            "placed": false,
            "paper_snapshot_sha256": digest_hex(request.digest),
            "request": native,
                "paper_health": {
                "initial": projected.health.initial,
                "after_bridges": projected.health.after_bridges,
                "scale": projected.health.husk.health_scale,
                "scaled": projected.health.husk.is_health_scaled,
            },
        });
        binding.add_to_reply(&mut reply);
        Ok(reply)
    }
}

fn paper_connection(hub: &Hub, binding: &Binding) -> Option<u32> {
    hub.guests
        .iter()
        .find(|guest| {
            guest.uuid == binding.owner
                && guest.expected
                && guest.participates_online()
                && guest.left_reason.is_empty()
                && ((guest.held
                    && !guest.placed
                    && guest
                        .arrival_handover
                        .is_none_or(|id| id == binding.handover))
                    || (guest.placed && guest.arrival_handover == Some(binding.handover)))
        })
        .map(|guest| guest.conn)
}

fn send_paper_reply(
    action: Activation,
    sender: mpsc::Sender<String>,
    result: Result<Value, String>,
) {
    let reply = result.unwrap_or_else(|error| json!({ "error": error }));
    // The request holds a SQL owner credential, including on malformed input. Log only its result.
    let (level, line) = arrivals::request_log(action.verb(), None, "Paper", &reply);
    super::super::log(level, &line);
    let _ = sender.send(reply.to_string());
}

fn paper_resume_reply(receipt: &receipt::Receipt, live: bool) -> Value {
    let mut reply = receipt.commit_reply(true, true);
    reply["retried"] = json!(live);
    reply["resumed"] = json!(!live);
    reply
}

fn paper_binding(
    settings: &arrivals::Settings,
    import: &PaperImport<'_>,
    destination: &str,
) -> Result<Binding, String> {
    let paper = &import.paper;
    Binding::from_confirm(
        settings,
        &format!(
            "{} {} {} {destination} {} {} {}",
            uuid_string(paper.owner),
            arrivals::handover_text(paper.handover),
            paper.source,
            import.record,
            import.request,
            u64::from(paper.revision),
        ),
    )
}

fn requested_paper_sections(
    roster: &BTreeMap<Identifier, u32>,
    sections: &BTreeMap<String, &[u8]>,
) -> Result<Vec<Value>, String> {
    let mut profiles = Vec::with_capacity(roster.len());
    let mut remaining = arrivals::MAX_REQUIRED_PROFILE_BYTES;
    for (id, version) in roster {
        let wire_id = id.to_string();
        let paper_name = match (wire_id.as_str(), *version) {
            ("slimefun:profile", 1) => "slimefun",
            _ => {
                return Err(format!(
                    "no Paper adapter for required profile {id} schema {version}"
                ));
            }
        };
        let bytes = sections
            .get(paper_name)
            .filter(|bytes| !bytes.is_empty())
            .ok_or_else(|| format!("Paper snapshot is missing required profile {paper_name}"))?;
        remaining = remaining
            .checked_sub(bytes.len())
            .ok_or("Paper plugin sections exceed the native profile budget")?;
        profiles.push(json!({ "id": wire_id, "version": version, "payload": base64(bytes) }));
    }
    Ok(profiles)
}

fn project_advancements(
    tree: &Tree,
    local: &Progress,
    snapshot: &BTreeMap<Identifier, BTreeMap<String, i64>>,
) -> Result<Progress, String> {
    let mut next = Progress::default();
    for (id, criteria) in snapshot {
        let name = id.to_string();
        let Some(advancement) = tree.get(&name) else {
            continue;
        };
        for criterion in &advancement.criteria {
            let Some(&millis) = criteria.get(&criterion.name) else {
                continue;
            };
            let date = match local
                .criteria
                .get(&name)
                .and_then(|local| local.get(&criterion.name))
            {
                Some(date) => date.clone(),
                None => DateTime::<Utc>::from_timestamp_millis(millis)
                    .ok_or("Paper advancement timestamp is out of range")?
                    .format("%Y-%m-%d %H:%M:%S %z")
                    .to_string(),
            };
            next.grant(&name, &criterion.name, &date);
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::super::super::{BanList, IpBanList, Lists, OpList, ServerState, WhiteList};
    use super::super::resume_tests::{
        KEY, forwarded_login, read_held_view, read_resumed_view, wait_for_join,
    };
    use super::*;
    use horizon_client::{
        items::ItemStack,
        lan::source_handover::{Handover, Journal},
    };
    use horizon_core::nbt::Compound;
    use std::{
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::Duration,
    };

    #[derive(Default)]
    struct SlimefunReceiver;

    #[mod_api::horizon_mod]
    impl mod_api::Hooks for SlimefunReceiver {
        fn player_transfer_profile_stage(
            &self,
            _: &dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerTransferProfileStageEvent<'_>,
        ) {
            assert_eq!(event.owner, 12);
            assert_eq!(event.id.to_string(), "slimefun:profile");
            assert_eq!(event.schema, 1);
            assert_eq!(event.payload, b"SFP1");
            *event.reply = Some(Ok(Compound::new()));
        }
    }

    fn paper_settings() -> arrivals::Settings {
        arrivals::Settings {
            backend_id: Some("horizon".into()),
            allowed_sources: ["paper".into()].into(),
            required_profiles: [(Identifier::parse_namespaced("slimefun:profile").unwrap(), 1)]
                .into(),
            ..arrivals::Settings::default()
        }
    }

    const OWNER_TOKEN: &str = "00000000-0000-0000-0000-000000000055";

    fn test_paper_request(
        game: &Game<'_>,
        args: &str,
        action: Activation,
    ) -> Result<
        (
            PendingPaperActivation,
            mpsc::Sender<Result<(), String>>,
            mpsc::Receiver<String>,
        ),
        String,
    > {
        let (send, response) = mpsc::channel();
        let (intent, check) =
            game.submit_paper_activation(args, action, |request, destination, token| {
                assert_eq!(request.owner, 12);
                assert_eq!(request.source, "paper");
                assert_eq!(destination, "horizon");
                assert_eq!(token, 0x55);
                Ok(AuthorityCheck {
                    deadline: Instant::now() + Duration::from_secs(8),
                    response,
                })
            })?;
        let (reply, result) = mpsc::channel();
        Ok((
            PendingPaperActivation {
                intent,
                check,
                reply,
            },
            send,
            result,
        ))
    }

    fn result(receive: mpsc::Receiver<String>) -> Value {
        serde_json::from_str(
            &receive
                .try_recv()
                .expect("the control request was completed"),
        )
        .unwrap()
    }

    fn with_authority(
        game: &mut Game<'_>,
        command: &str,
        action: Activation,
        authority: Result<(), String>,
    ) -> Value {
        match test_paper_request(game, command, action) {
            Err(error) => json!({ "error": error }),
            Ok((pending, send, receive)) => {
                send.send(authority).unwrap();
                game.poll_paper_request(&mut Some(pending));
                result(receive)
            }
        }
    }

    fn authorized(game: &mut Game<'_>, command: &str, action: Activation) -> Value {
        with_authority(game, command, action, Ok(()))
    }

    fn control_state(world: &Path, paper_activation: PendingPaperActivation) -> ServerState {
        let lists = Lists {
            ops: OpList::new(world.join("ops.json")),
            white: WhiteList::new(world.join("whitelist.json")),
            bans: BanList::new(world.join("banned-players.json")),
            ip_bans: IpBanList::new(world.join("banned-ips.json")),
            use_whitelist: false,
            enforce_whitelist: false,
            op_level: 4,
            player_idle_timeout: 0,
            stop: false,
            autosave: true,
            whitelist_bypass: Vec::new(),
        };
        ServerState {
            lists: Arc::new(Mutex::new(lists)),
            console: mpsc::channel().1,
            paper_activation: Some(paper_activation),
            empty_ticks: 0,
            pause_when_empty_seconds: 0,
            ticks: Default::default(),
            paused: false,
            broadcast_console: false,
            broadcast_rcon: false,
            op_level: 4,
            report: None,
            memory_due: None,
            perf: horizon_client::server_perf::ServerPerf::new(0.0, 250.0),
            perf_epoch: Instant::now(),
            horizon: horizon_client::server_perf::horizon_options(0, 0, false),
            view: (2, 2),
            compression: -1,
            max_players: 2,
            game_mode: 0,
            force_game_mode: false,
        }
    }

    #[test]
    #[ignore = "daily"]
    fn paper_snapshot_prepares_then_stages_and_commits_with_an_exact_receipt() {
        let fixtures = [
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../horizon-items/tests/fixtures/husksync/husk-effects-attributes-26.2.json"
            ))
            .as_slice(),
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../horizon-items/tests/fixtures/husksync/husk-records-26.2.json"
            ))
            .as_slice(),
        ];
        let mut documents = fixtures
            .into_iter()
            .map(|bytes| serde_json::from_slice::<Value>(bytes).unwrap());
        let mut snapshot = documents.next().unwrap();
        snapshot["data"].as_object_mut().unwrap().extend(
            documents.next().unwrap()["data"]
                .as_object()
                .unwrap()
                .clone(),
        );
        let packed = serde_json::to_vec(&snapshot).unwrap();
        let envelope = SnapshotEnvelope {
            health: 87.25,
            recipes: Default::default(),
            sections: [("slimefun".into(), b"SFP1".as_slice())].into(),
            husk: &packed,
        }
        .encode(MAX_SNAPSHOT_BYTES)
        .unwrap();
        let world = tempfile::tempdir().unwrap();
        let digest = pm_sync_spool::write(world.path(), &envelope).unwrap();
        let owner = 12;
        let uuid = uuid_string(owner);
        let handover = arrivals::handover_text(31);
        let paper = format!("{uuid} {handover} paper 9 {} json", digest_hex(digest));
        let file = playerdata::path(world.path(), owner);
        let resources = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let mut committed_proof = None;
        crate::game::native_guest_turn_tests::with_guests_configured(
            resources.clone(),
            |net| net.forwarding_secret = Some(b"local test forwarding".to_vec()),
            |game| {
                game.world_root = world.path().to_owned();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(arrivals::Transfers::new(paper_settings()));
                hub.update_guest(12, |guest| {
                    guest.placed = false;
                    guest.held = true;
                    guest.expected = true;
                    guest.xp_level = 2;
                });
                assert!(
                    game.player_request(
                        "player-profile-prepare-paper",
                        &paper.replace(" paper ", " unknown ")
                    )["error"]
                        .is_string()
                );
                let prepared = game.player_request("player-profile-prepare-paper", &paper);
                assert_eq!(prepared["prepared"], true, "{prepared}");
                assert_eq!(prepared["placed"], false);
                assert_eq!(prepared["authority_revision"], 9);
                assert_eq!(prepared["paper_snapshot_sha256"], digest_hex(digest));
                assert_eq!(prepared["paper_health"]["after_bridges"], 87.25);
                assert!(prepared.get("request").is_none());
                assert_eq!(
                    game.player_request("player-profile-prepare-paper", &paper),
                    prepared
                );
                assert!(!file.exists());
                let record_digest = prepared["record_sha256"].as_str().unwrap();
                let request_digest = prepared["request_sha256"].as_str().unwrap();
                let apply = format!("{paper} {record_digest} {request_digest}");
                let confirm =
                    format!("{uuid} {handover} paper horizon {record_digest} {request_digest} 9");
                let mods = vec![mod_api::ModEntry::new(
                    "slimefun",
                    Box::new(SlimefunReceiver),
                )]
                .into();
                mod_platform::testing::with_mods(mods, || {
                    let wrong = format!("{paper} {} {request_digest}", "0".repeat(64));
                    assert!(
                        game.player_request("player-profile-import-paper", &wrong)["error"]
                            .is_string()
                    );
                    let raw_request = game
                        .project_paper_profile(&PaperRequest::parse(&paper).unwrap())
                        .unwrap()["request"]
                        .clone();
                    let native =
                        game.player_request("player-import", &format!("{uuid} {raw_request}"));
                    assert!(
                        native["error"]
                            .as_str()
                            .unwrap()
                            .contains("dedicated Paper import"),
                        "{native}"
                    );
                    let wire = format!("{apply} {OWNER_TOKEN}");
                    let (reply, _receive) = std::sync::mpsc::channel();
                    let control = horizon_client::control::Server::local();
                    control
                        .sender()
                        .send(horizon_client::control::Request {
                            line: format!("player-profile-commit-paper {wire}"),
                            reply,
                        })
                        .unwrap();
                    // The new authoritative head is observed at dispatch time, after queueing.
                    let database_revision = 10;
                    let late = control.try_recv().unwrap();
                    let (_, delayed) = late.line.split_once(' ').unwrap();
                    assert!(
                        database_revision
                            > u64::from(PaperImport::parse(&apply).unwrap().paper.revision)
                    );
                    let stale = with_authority(
                        game,
                        delayed,
                        Activation::Commit,
                        Err("PM-Sync source does not own the expected revision".into()),
                    );
                    assert!(
                        stale["error"]
                            .as_str()
                            .unwrap()
                            .contains("expected revision")
                    );
                    let interrupted = with_authority(
                        game,
                        &wire,
                        Activation::Commit,
                        Err("the Paper database did not acknowledge its authority check".into()),
                    );
                    assert!(
                        interrupted["error"]
                            .as_str()
                            .unwrap()
                            .contains("did not acknowledge")
                    );
                    #[cfg(not(feature = "pm-sync-mysql"))]
                    {
                        let (reply, receive) = mpsc::channel();
                        game.start_paper_request(
                            Activation::Commit.verb(),
                            &wire,
                            reply,
                            &mut None,
                        );
                        assert!(
                            result(receive)["error"]
                                .as_str()
                                .unwrap()
                                .contains("built without PM-Sync MySQL")
                        );
                    }
                    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert!(guest.held && !guest.placed && !guest.imported);
                    assert!(!file.exists());

                    let (pending, answer, receive) =
                        test_paper_request(game, &wire, Activation::Commit).unwrap();
                    let mut state = control_state(world.path(), pending);
                    let control = horizon_client::control::Server::local();
                    let (reply, status) = mpsc::channel();
                    control
                        .sender()
                        .send(horizon_client::control::Request {
                            line: "transfer-status".into(),
                            reply,
                        })
                        .unwrap();
                    game.control = Some(control);
                    game.poll_server_control(&mut state);
                    assert_eq!(result(status)["held"], 1);
                    assert!(state.paper_activation.is_some());
                    assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
                    assert!(!file.exists());
                    // Disconnect the original slot and replace it with another connection for the same UUID.
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    let index = hub.guests.connection_index(12).unwrap();
                    let mut guest = hub.guests.remove(index);
                    guest.conn = 21;
                    hub.guests.insert(index, guest);
                    answer.send(Ok(())).unwrap();
                    game.poll_server_control(&mut state);
                    assert!(
                        result(receive)["error"]
                            .as_str()
                            .unwrap()
                            .contains("connection changed")
                    );
                    assert!(!file.exists());
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    let mut guest = hub.guests.remove(index);
                    guest.conn = 12;
                    hub.guests.insert(index, guest);

                    let (mut pending, answer, receive) =
                        test_paper_request(game, &wire, Activation::Commit).unwrap();
                    pending.check.deadline = Instant::now();
                    state.paper_activation = Some(pending);
                    answer.send(Ok(())).unwrap();
                    game.poll_server_control(&mut state);
                    assert!(
                        result(receive)["error"]
                            .as_str()
                            .unwrap()
                            .contains("timed out")
                    );
                    assert!(!file.exists());

                    let (pending, answer, receive) =
                        test_paper_request(game, &wire, Activation::Commit).unwrap();
                    state.paper_activation = Some(pending);
                    state.lists.lock().unwrap().stop = true;
                    answer.send(Ok(())).unwrap();
                    game.poll_server_control(&mut state);
                    assert_eq!(result(receive)["error"], arrivals::STOPPING);
                    assert!(!file.exists());
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .transfers
                        .as_mut()
                        .unwrap()
                        .stopping = false;
                    game.control = None;

                    let staged = game.player_request("player-profile-import-paper", &apply);
                    assert_eq!(staged["placed"], false, "{staged}");
                    assert_eq!(staged["ok"], true);
                    assert!(!file.exists());
                    let retry = game.player_request("player-profile-import-paper", &apply);
                    assert_eq!(retry["retried"], true, "{retry}");
                    assert_eq!(retry["placed"], false);
                    let bypass = game.player_request("player-confirm", &confirm);
                    assert!(
                        bypass["error"]
                            .as_str()
                            .unwrap()
                            .contains("current PM-Sync claim")
                    );
                    assert!(!file.exists());
                    let placed = authorized(game, &wire, Activation::Commit);
                    assert_eq!(placed["currently_placed"], true, "{placed}");
                    assert_eq!(placed["paper_snapshot_sha256"], digest_hex(digest));
                    let saved = horizon_client::save::read_dat(&file).unwrap();
                    assert!(saved.get(receipt::PAPER_AUTHORITY_KEY).is_some());
                    assert_eq!(saved.get("SelectedItemSlot"), Some(&Tag::Int(4)));
                    assert_eq!(
                        saved.get("foodExhaustionLevel"),
                        Some(&Tag::Float((-3.25f32).to_bits()))
                    );
                    assert_eq!(
                        saved.get("Pos"),
                        Some(&Tag::List(vec![
                            Tag::Double(3.0f64.to_bits()),
                            Tag::Double(64.0f64.to_bits()),
                            Tag::Double(5.0f64.to_bits()),
                        ]))
                    );
                    let exact_file = std::fs::read(&file).unwrap();
                    let retry = game.player_request("player-profile-import-paper", &apply);
                    assert_eq!(retry["already_committed"], true);
                    let unrelated = apply.replace(&digest_hex(digest), &"f".repeat(64));
                    assert!(
                        game.player_request("player-profile-import-paper", &unrelated)["error"]
                            .is_string()
                    );
                    assert_eq!(std::fs::read(&file).unwrap(), exact_file);
                });
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.update_guest(12, |guest| {
                    guest.place([12.0, 70.0, 24.0]);
                    guest.dimension = "minecraft:the_nether".into();
                    guest.xp_level = 41;
                    guest.inventory.fill(None);
                    guest.armor.fill(None);
                    guest.offhand = None;
                    guest.equipment = Default::default();
                    guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
                    playerdata::save_now(guest, world.path()).unwrap();
                });
                committed_proof = Some((confirm, apply));
            },
        );

        let (confirm, apply) = committed_proof.unwrap();
        let resume = format!("{apply} {OWNER_TOKEN}");
        let latest = std::fs::read(&file).unwrap();
        let Tag::Compound(mut legacy) = horizon_client::save::read_dat(&file).unwrap() else {
            unreachable!()
        };
        legacy.remove(&receipt::PAPER_AUTHORITY_KEY.into()).unwrap();
        horizon_client::save::write_dat_durable(&file, Tag::Compound(legacy.clone())).unwrap();
        let legacy_bytes = std::fs::read(&file).unwrap();
        let spool = world.path().join("horizon-pm-sync-spool");
        let frozen_spool = world.path().join("unavailable-paper-spool");
        std::fs::rename(&spool, &frozen_spool).unwrap();
        let journal = Arc::new(Journal::open(world.path()).unwrap());
        let transfers = arrivals::Transfers::new(paper_settings());
        let table = transfers.expected.clone();
        crate::game::native_guest_turn_tests::with_guests_configured(
            resources,
            |net| {
                net.listen = true;
                net.bind = Some(std::net::Ipv4Addr::LOCALHOST.into());
                net.forwarding_secret = Some(KEY.to_vec());
                net.arrivals = Some(table);
                net.source_handovers = Some(journal.clone());
            },
            |game| {
                game.world_root = world.path().to_owned();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(transfers);
                hub.guests.remove(hub.guests.connection_index(12).unwrap());
                let port = hub.lan.port;
                assert!(forwarded_login(port, false).is_err());
                assert!(authorized(game, &resume, Activation::Resume)["error"].is_string());
                let (mut historical, published, historical_reply) =
                    test_paper_request(game, &resume, Activation::Commit).unwrap();
                assert!(historical.intent.connection.is_none());
                historical.check.deadline = Instant::now() + Duration::from_secs(30);
                assert_eq!(game.player_request("player-expect", &uuid)["ok"], true);
                let mut client = forwarded_login(port, true).unwrap();
                let conn = wait_for_join(game);
                assert!(!game.lan_guest_joined(conn, &[], [0.5, 64.0, 0.5]));
                let blindness = read_held_view(&mut client);
                published.send(Ok(())).unwrap();
                game.poll_paper_request(&mut Some(historical));
                let historical_reply = result(historical_reply);
                assert_eq!(
                    historical_reply["already_committed"], true,
                    "{historical_reply}"
                );
                assert_eq!(
                    historical_reply["currently_placed"], false,
                    "{historical_reply}"
                );
                assert!(game.lan.as_ref().unwrap().hub.guest(conn).unwrap().held);

                let old_unsigned = format!("{confirm} {}", digest_hex(digest));
                let (reply, receive) = mpsc::channel();
                game.start_paper_request(
                    Activation::Resume.verb(),
                    &old_unsigned,
                    reply,
                    &mut None,
                );
                assert!(result(receive)["error"].is_string());
                let wrong = resume.replace(&digest_hex(digest), &"0".repeat(64));
                assert!(authorized(game, &wrong, Activation::Resume)["error"].is_string());
                let interrupted = with_authority(
                    game,
                    &resume,
                    Activation::Resume,
                    Err("the Paper database did not acknowledge its authority check".into()),
                );
                assert!(
                    interrupted["error"]
                        .as_str()
                        .unwrap()
                        .contains("did not acknowledge")
                );
                assert!(game.lan.as_ref().unwrap().hub.guest(conn).unwrap().held);
                assert_eq!(std::fs::read(&file).unwrap(), legacy_bytes);
                let history = game.player_request("player-profile-import-paper", &apply);
                assert_eq!(history["currently_placed"], false, "{history}");
                assert!(file.with_extension("dat_old").is_file());
                std::fs::remove_file(&file).unwrap();
                assert!(authorized(game, &resume, Activation::Resume)["error"].is_string());
                std::fs::write(&file, b"broken data despite a correct backup").unwrap();
                assert!(authorized(game, &resume, Activation::Resume)["error"].is_string());

                let mut superseded = legacy.clone();
                let Some(Tag::Compound(profiles)) =
                    superseded.get_mut(&"HorizonTransferProfiles".into())
                else {
                    unreachable!()
                };
                let Some(Tag::Compound(marker)) = profiles.get_mut(&"slimefun:profile".into())
                else {
                    unreachable!()
                };
                marker.insert(
                    "generation".into(),
                    Tag::ByteArray(32u128.to_be_bytes().to_vec()),
                );
                horizon_client::save::write_dat_durable(&file, Tag::Compound(superseded)).unwrap();
                assert!(
                    authorized(game, &resume, Activation::Resume)["error"]
                        .as_str()
                        .unwrap()
                        .contains("required profile")
                );
                std::fs::write(&file, &legacy_bytes).unwrap();

                let resumed = authorized(game, &resume, Activation::Resume);
                assert_eq!(resumed["resumed"], true, "{resumed}");
                assert_eq!(resumed["already_committed"], true);
                assert_eq!(resumed["retried"], false);
                assert_eq!(resumed["currently_placed"], true);
                read_resumed_view(&mut client, blindness);
                let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
                assert!(guest.placed_in("minecraft:the_nether"));
                assert_eq!(guest.position, [12.0, 70.0, 24.0]);
                assert_eq!(guest.xp_level, 41);
                assert_eq!(std::fs::read(&file).unwrap(), legacy_bytes);

                std::fs::write(&file, &latest).unwrap();
                let retried = authorized(game, &resume, Activation::Resume);
                assert_eq!(retried["resumed"], false, "{retried}");
                assert_eq!(retried["retried"], true);
                assert_eq!(std::fs::read(&file).unwrap(), latest);

                // A later successful strict Paper commit supersedes the old marker and active generation.
                std::fs::rename(&frozen_spool, &spool).unwrap();
                game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                    guest.placed = false;
                    guest.held = true;
                    guest.arrival_handover = None;
                });
                let next = arrivals::handover_text(32);
                let later = format!("{uuid} {next} paper 10 {} json", digest_hex(digest));
                let prepared = game.player_request("player-profile-prepare-paper", &later);
                assert_eq!(prepared["prepared"], true, "{prepared}");
                let record = prepared["record_sha256"].as_str().unwrap();
                let request = prepared["request_sha256"].as_str().unwrap();
                let next_wire = format!("{later} {record} {request} {OWNER_TOKEN}");
                mod_platform::testing::with_mods(
                    vec![mod_api::ModEntry::new(
                        "slimefun",
                        Box::new(SlimefunReceiver),
                    )]
                    .into(),
                    || {
                        let staged = game.player_request(
                            "player-profile-import-paper",
                            &format!("{later} {record} {request}"),
                        );
                        assert_eq!(staged["placed"], false, "{staged}");
                        let placed = authorized(game, &next_wire, Activation::Commit);
                        assert_eq!(placed["currently_placed"], true, "{placed}");
                    },
                );
                game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                    guest.placed = false;
                    guest.held = true;
                    guest.arrival_handover = None;
                });
                assert!(authorized(game, &resume, Activation::Resume)["error"].is_string());

                let outgoing = Handover {
                    owner,
                    id: 47,
                    destination: "next".into(),
                    record: b"exact outgoing record".to_vec(),
                    local_checkpoint: b"exact current checkpoint".to_vec(),
                };
                journal
                    .prepare_versioned(&outgoing, 11.try_into().unwrap())
                    .unwrap();
                let fenced = |game: &mut Game<'_>| {
                    let answer = authorized(game, &next_wire, Activation::Resume);
                    assert!(
                        answer["error"].as_str().unwrap().contains("fences"),
                        "{answer}"
                    );
                };
                fenced(game);
                journal
                    .retire(
                        owner,
                        outgoing.id,
                        &outgoing.destination,
                        outgoing.record_digest(),
                    )
                    .unwrap();
                fenced(game);
            },
        );
    }
}
