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
        arrivals, base64, playerdata,
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
use std::collections::BTreeMap;

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

impl Game<'_> {
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
        self.stage_exact_paper_profile(args)
            .unwrap_or_else(|error| json!({ "error": error }))
    }

    fn stage_exact_paper_profile(&mut self, args: &str) -> Result<Value, String> {
        let args = args.trim_end();
        let (before, expected_request) = args
            .rsplit_once(char::is_whitespace)
            .ok_or("Paper profile import needs a prepared record and request digest")?;
        let (paper, expected_record) = before
            .trim_end()
            .rsplit_once(char::is_whitespace)
            .ok_or("Paper profile import needs a prepared record and request digest")?;
        let request = PaperRequest::parse(paper)?;
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
        let binding = Binding::from_confirm(
            &transfer.settings,
            &format!(
                "{} {} {} {destination} {expected_record} {expected_request} {}",
                uuid_string(request.owner),
                arrivals::handover_text(request.handover),
                request.source,
                u64::from(request.revision)
            ),
        )?;
        if let Some(committed) = receipt::lookup(&self.world_root, &binding)? {
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
            if staged.binding != binding || staged.paper_snapshot_sha256 != Some(request.digest) {
                return Err("the Paper snapshot differs from its held profile stage".into());
            }
            let mut reply = json!({"ok": true, "placed": false, "retried": true,
                "paper_snapshot_sha256": digest_hex(request.digest) });
            binding.add_to_reply(&mut reply);
            return Ok(reply);
        }
        let mut projected = self.project_paper_profile(&request)?;
        if projected["record_sha256"].as_str() != Some(expected_record)
            || projected["request_sha256"].as_str() != Some(expected_request)
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
    use super::*;
    use horizon_core::nbt::Compound;
    use std::path::PathBuf;

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
        crate::game::native_guest_turn_tests::with_guests_configured(
            resources,
            |net| net.forwarding_secret = Some(b"local test forwarding".to_vec()),
            |game| {
                game.world_root = world.path().to_owned();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(arrivals::Transfers::new(arrivals::Settings {
                    backend_id: Some("horizon".into()),
                    allowed_sources: ["paper".into()].into(),
                    required_profiles: [(
                        Identifier::parse_namespaced("slimefun:profile").unwrap(),
                        1,
                    )]
                    .into(),
                    ..arrivals::Settings::default()
                }));
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
                            .contains("food exhaustion"),
                        "{native}"
                    );
                    let staged = game.player_request("player-profile-import-paper", &apply);
                    assert_eq!(staged["placed"], false, "{staged}");
                    assert_eq!(staged["ok"], true);
                    assert!(!file.exists());
                    let retry = game.player_request("player-profile-import-paper", &apply);
                    assert_eq!(retry["retried"], true, "{retry}");
                    assert_eq!(retry["placed"], false);
                    let placed = game.player_request("player-confirm", &confirm);
                    assert_eq!(placed["currently_placed"], true, "{placed}");
                    assert_eq!(placed["paper_snapshot_sha256"], digest_hex(digest));
                    let saved = horizon_client::save::read_dat(&file).unwrap();
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
            },
        );
    }
}
