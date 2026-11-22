//! Source commands for an owned, durable player hand-over.

use super::transfer::receipt::{Hidden, digest_hex};
use crate::game::{Dim, Game};
use horizon_client::lan::{
    arrivals, base64, playerdata,
    source_handover::{Handover, Journal, Phase},
};
use horizon_core::nbt::{Limits, NamedTag, write_named};
use horizon_protocol::packets::play::{self, Clientbound as C};
use horizon_server_files::transfer_routes::valid_backend_id;
use serde_json::{Value, json};
use std::time::Instant;

impl Game<'_> {
    /// Close the guest's menus exactly as on departure, including the player's
    /// own crafting grid. The returned index belongs to the same connection.
    pub(super) fn settle_transfer_menus(&mut self, conn: u32) -> Result<usize, String> {
        let host = self.lan.as_ref().ok_or("the server is not open")?;
        let guest = host.hub.guest(conn).ok_or("that connection has left")?;
        let window = guest.menu.as_ref().map(|menu| menu.window);
        let destination = Dim::from_id(&guest.dimension);
        let original = self.dim;
        self.enter_level(destination)
            .map_err(|error| format!("the player's level could not be opened: {error}"))?;
        self.guest_close_all(conn);
        self.lan_crafting_closed(conn, Some(0));
        if let Err(error) = self.enter_level(original) {
            log::error!("could not reopen {}: {error}", original.id());
        }
        let hub = &mut self.lan.as_mut().ok_or("the server is not open")?.hub;
        let index = hub
            .guests
            .connection_index(conn)
            .ok_or("that connection has left")?;
        // An export may fail or be read-only, so even a client that stays must
        // close the same menu and see the reconstructed inventory.
        if let Some(container) = window {
            hub.send_to(conn, &C::ContainerClose(play::ContainerClose { container }));
        }
        hub.send_inventory(index);
        Ok(index)
    }

    pub(super) fn transfer_source_records(&self, index: usize) -> arrivals::Records {
        let host = self.lan.as_ref().expect("the server is open");
        let guest = &host.hub.guests[index];
        let advancements = if self.adv.tree.list.is_empty() {
            Value::Null
        } else {
            guest
                .advancements
                .to_json(&self.adv.tree, horizon_world::DATA_VERSION)
        };
        let stats = host
            .play
            .get(&guest.conn)
            .map_or(Value::Null, |play| play.stats.to_json());
        arrivals::Records {
            advancements,
            stats,
        }
    }

    pub(super) fn run_profile_source_request(&mut self, verb: &str, args: &str) -> Value {
        self.profile_source_request(verb, args)
            .unwrap_or_else(|error| json!({ "error": error }))
    }

    fn profile_source_request(&mut self, verb: &str, args: &str) -> Result<Value, String> {
        let mut fields = args.split_whitespace();
        let owner = fields
            .next()
            .and_then(arrivals::parse_uuid)
            .filter(|owner| *owner != 0)
            .ok_or("a source hand-over needs the player's unique id")?;
        let id = fields
            .next()
            .and_then(arrivals::parse_handover)
            .filter(|id| *id != 0)
            .ok_or("a source hand-over needs a nonzero hand-over id")?;
        let destination = fields
            .next()
            .filter(|destination| valid_backend_id(destination))
            .ok_or("a source hand-over needs a valid destination backend id")?;
        let digest = fields.next();
        if fields.next().is_some() || (verb == "player-profile-retire") != digest.is_some() {
            return Err("source export or status takes a unique id, hand-over id and destination; retirement also requires the record digest".into());
        }

        let journal = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.lan.shared.config.source_handovers.clone())
            .ok_or("the source hand-over journal is unavailable")?;
        if verb == "player-profile-export" && journal.fenced_handover(owner).is_none() {
            let (handover, phase) =
                self.prepare_profile_source(&journal, owner, id, destination)?;
            return source_reply(&handover, phase, true);
        }

        let (phase, handover) = journal
            .recover(owner, id)
            .map_err(|error| format!("could not recover the matching source hand-over: {error}"))?;
        if handover.destination != destination {
            return Err("the source hand-over belongs to another destination".into());
        }
        if let Some(digest) = digest {
            let expected = handover.record_digest();
            if digest_hex(expected) != digest {
                return Err("the source hand-over record digest does not match".into());
            }
            journal
                .retire(owner, id, destination, expected)
                .map_err(|error| {
                    format!("could not retire the matching source hand-over: {error}")
                })?;
            self.lan
                .as_mut()
                .expect("a source journal belongs to the server")
                .hub
                .detach_retired_profile_source(owner);
            return source_reply(&handover, Phase::Retired, false);
        }
        source_reply(&handover, phase, verb == "player-profile-export")
    }

    fn prepare_profile_source(
        &mut self,
        journal: &Journal,
        owner: u128,
        id: u128,
        destination: &str,
    ) -> Result<(Handover, Phase), String> {
        let host = self.lan.as_ref().ok_or("the server is not open")?;
        let transfers = host.hub.transfers.as_ref().ok_or(arrivals::OFF)?;
        let source = transfers
            .settings
            .backend_id
            .as_ref()
            .filter(|source| source.as_str() != destination)
            .ok_or("the source backend must have its own configured id")?
            .clone();
        let roster = transfers.settings.required_profiles.clone();
        if roster.is_empty() {
            return Err("the source must configure a required profile roster".into());
        }
        if transfers.stopping
            && transfers
                .stop_wait
                .as_ref()
                .is_none_or(|wait| !wait.may_freeze(Instant::now()))
        {
            return Err(arrivals::STOPPING.into());
        }
        let index = arrivals::check_exportable(&host.hub, owner)?;
        let guest = &host.hub.guests[index];
        let conn = guest.conn;
        if guest.is_fake()
            || !guest.health.is_finite()
            || guest.health <= 0.0
            || guest.handover.is_some()
            || self.on_server_trip(conn)
        {
            return Err("that player is already moving or is not a live source player".into());
        }

        self.finish_player_autosaves(true)
            .map_err(|error| format!("could not finish the source player saves: {error}"))?;
        let index = self.settle_transfer_menus(conn)?;
        let hub = &self.lan.as_ref().expect("the server is open").hub;
        let guest = &hub.guests[index];
        if arrivals::check_exportable(hub, owner)? != index
            || !guest.health.is_finite()
            || guest.health <= 0.0
            || self.on_server_trip(conn)
        {
            return Err("the player changed sessions while closing the source menus".into());
        }
        let records = self.transfer_source_records(index);
        let mut sections = Vec::with_capacity(roster.len());
        let mut remaining = arrivals::MAX_REQUIRED_PROFILE_BYTES;
        for (id, schema) in roster {
            let mut reply = None;
            self.native_player_transfer_profile_capture(
                mod_api::__private::PlayerTransferProfileCaptureCall {
                    mod_id: id.namespace(),
                    owner,
                    id: &id,
                    schema,
                    reply: &mut reply,
                },
            );
            let payload = reply.ok_or_else(|| {
                format!("no approved source collector accepted required profile {id}")
            })??;
            if payload.is_empty() || payload.len() > remaining {
                return Err(format!(
                    "source profile {id} exceeds the transfer profile budget or is empty"
                ));
            }
            remaining -= payload.len();
            sections.push(
                json!({ "id": id.to_string(), "version": schema, "payload": base64(&payload) }),
            );
        }
        let hub = &mut self.lan.as_mut().expect("the server is open").hub;
        let local_nbt = write_named(
            &NamedTag {
                name: "".into(),
                tag: playerdata::player_tag(&hub.guests[index]),
            },
            Limits::saved(),
        )
        .map_err(|error| error.to_string())?;
        let local_checkpoint = serde_json::to_vec(&json!({
            "player": base64(&local_nbt),
            "advancements": records.advancements,
            "stats": records.stats,
        }))
        .map_err(|error| error.to_string())?;
        // The portable record strips both this world's committed profile pointers
        // and its receipt history. The source checkpoint retains them for recovery.
        let hidden = hub.guests.update(index, Hidden::take);
        let portable = arrivals::export_record(hub, index, false, records);
        hub.guests.update(index, |guest| hidden.restore(guest));
        let mut record = portable?;
        record["version"] = json!(2);
        record["handover"] = json!(arrivals::handover_text(id));
        record["source"] = json!(source);
        record["destination"] = json!(destination);
        record["profiles"] = json!(sections);
        let record = serde_json::to_vec(&record).map_err(|error| error.to_string())?;
        // Reserve room for destination import options and control framing.
        if record.len() + (64 << 10) > horizon_client::control::MAX_LINE as usize {
            return Err("the source record exceeds the control request budget".into());
        }
        let handover = Handover {
            owner,
            id,
            destination: destination.into(),
            record,
            local_checkpoint,
        };
        // Before the irreversible ownership step, checkpoint the ordinary player
        // file and counters as the local fallback for an unpublished journal.
        let host = self.lan.as_ref().expect("the server is open");
        let guest = &host.hub.guests[index];
        self.write_arrival_records(host, guest)?;
        playerdata::save_now(guest, &self.world_root)?;
        let result = journal.prepare(&handover);
        if journal.fenced_handover(owner) == Some(id) {
            let hub = &mut self.lan.as_mut().expect("the server is open").hub;
            if !hub.park_profile_source(index, id) {
                hub.kick(
                    conn,
                    horizon_protocol::text::translatable("multiplayer.disconnect.generic", &[]),
                );
            }
        }
        let phase =
            result.map_err(|error| format!("could not prepare the source hand-over: {error}"))?;
        Ok((handover, phase))
    }
}

fn source_reply(handover: &Handover, phase: Phase, include_record: bool) -> Result<Value, String> {
    let record: Value = serde_json::from_slice(&handover.record)
        .map_err(|error| format!("the stored hand-over record is invalid: {error}"))?;
    let mut reply = json!({
        "ok": true,
        "handover": arrivals::handover_text(handover.id),
        "source": record["source"],
        "destination": handover.destination,
        "record_sha256": digest_hex(handover.record_digest()),
        "phase": match phase { Phase::Pending => "pending", Phase::Retired => "retired" },
    });
    if include_record {
        reply["record"] = record;
    }
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Hooks, Identifier, PlayerTransferProfileCaptureEvent, ServerHost};
    use std::{path::PathBuf, sync::Arc};

    struct Capture;

    #[mod_api::__subscriptions]
    impl Hooks for Capture {
        fn player_transfer_profile_capture(
            &self,
            host: &dyn ServerHost,
            event: &mut PlayerTransferProfileCaptureEvent<'_>,
        ) {
            assert_eq!(event.owner, 12);
            assert_eq!(event.id.to_string(), "example:profile");
            assert_eq!(event.schema, 1);
            assert!(host.player(event.owner).is_some());
            *event.reply = Some(Ok(b"example owned data".to_vec()));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn source_dispatch_reserves_and_retires_only_the_captured_record() {
        let world = tempfile::tempdir().unwrap();
        let journal = Arc::new(Journal::open(world.path()).unwrap());
        let test_resources = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let owner = horizon_protocol::crypto::uuid_string(12);
        let id = arrivals::handover_text(19);
        let route = format!("{owner} {id} east");
        crate::game::native_guest_turn_tests::with_guests_configured(
            test_resources,
            |net| net.source_handovers = Some(journal.clone()),
            |game| {
                game.world_root = world.path().to_owned();
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(arrivals::Transfers::new(arrivals::Settings {
                    backend_id: Some("west".into()),
                    required_profiles: [(
                        Identifier::parse_namespaced("example:profile").unwrap(),
                        1,
                    )]
                    .into(),
                    ..arrivals::Settings::default()
                }));
                let unrelated = vec![mod_api::ModEntry::new("other", Box::new(Capture))].into();
                mod_platform::testing::with_mods(unrelated, || {
                    let reply = game.player_request("player-profile-export", &route);
                    assert!(
                        reply["error"]
                            .as_str()
                            .unwrap()
                            .contains("no approved source")
                    );
                });
                assert_eq!(journal.fenced_handover(12), None);
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.update_guest(12, |guest| guest.health = 0.0);
                assert!(game.player_request("player-profile-export", &route)["error"].is_string());
                assert_eq!(journal.fenced_handover(12), None);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.health = 20.0);
                let mods = vec![mod_api::ModEntry::new("example", Box::new(Capture))].into();
                let published = mod_platform::testing::with_mods(mods, || {
                    game.player_request("player-profile-export", &route)
                });
                assert_eq!(published["phase"], "pending", "{published}");
                assert_eq!(
                    published["record"]["profiles"][0]["payload"],
                    base64(b"example owned data")
                );
                assert_eq!(published["record"]["source"], "west");
                assert_eq!(journal.fenced_handover(12), Some(19));
                assert_eq!(
                    game.player_request("player-profile-export", &route),
                    published
                );
                let bad = format!("{route} {}", "00".repeat(32));
                assert!(game.player_request("player-profile-retire", &bad)["error"].is_string());
                assert_eq!(journal.recover(12, 19).unwrap().0, Phase::Pending);

                // Existing ownership can be reconciled after transfer is disabled.
                game.lan.as_mut().unwrap().hub.transfers = None;
                assert_eq!(
                    game.player_request("player-profile-status", &route)["phase"],
                    "pending"
                );
                let proof = format!("{route} {}", published["record_sha256"].as_str().unwrap());
                let retired = game.player_request("player-profile-retire", &proof);
                assert_eq!(retired["phase"], "retired");
                assert_eq!(
                    game.player_request("player-profile-retire", &proof),
                    retired
                );
                assert!(game.lan.as_ref().unwrap().hub.guest(12).is_none());
                assert_eq!(
                    Journal::open(world.path())
                        .unwrap()
                        .recover(12, 19)
                        .unwrap()
                        .0,
                    Phase::Retired
                );
            },
        );
    }
}
