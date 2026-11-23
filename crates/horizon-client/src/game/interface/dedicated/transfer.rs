//! A proxy's requests on the control endpoint, for a player it moves between two servers:
//! `player-expect <uuid> [seconds]` (the target is told who is coming; its login is held),
//! `player-drop <uuid>` (the target forgets it, and lets a held player go),
//! `player-export <uuid> [freeze]` (the source answers with the player's record) and
//! `player-release <uuid> <hand-over> moved [announce] | abort` (the source lets the player go),
//! `player-import <uuid> <json>` and `player-confirm <uuid> <hand-over>` (the target takes the
//! record and lets the held player in), and `transfer-status` (the counts a proxy reads before
//! it sends players here). They run
//! between two ticks, on the server thread, so the player is never caught half-way through a
//! tick. Legacy transfers and new source captures require `[transfer]`. The strict source's
//! status and retirement commands can reconcile existing journal entries after a config change.
use crate::game::Game;
use horizon_client::lan::{
    arrivals, import, playerdata,
    source_handover::{CurrentReturn, ReturnAuthorization},
};
use serde_json::{Value, json};
use std::time::Instant;

#[path = "transfer_paper.rs"]
mod paper;

#[path = "transfer_receipt.rs"]
pub(super) mod receipt;

#[cfg(test)]
#[path = "transfer_resume_tests.rs"]
mod resume_tests;

impl Game<'_> {
    /// A proxy's `player-*` request; the reply is one JSON line, `{"ok": true, ...}` or
    /// `{"error": "..."}`. Each request writes one line to the log with what became of it
    /// (`arrivals::request_log`): a hand-over that failed leaves a trace.
    pub(super) fn player_request(&mut self, verb: &str, args: &str) -> Value {
        // The name is read before the request runs: a release or a drop takes the player away.
        let name = args
            .split_whitespace()
            .next()
            .and_then(arrivals::parse_uuid)
            .and_then(|uuid| {
                self.lan
                    .as_ref()?
                    .hub
                    .guests
                    .iter()
                    .find(|g| g.uuid == uuid)
                    .map(|g| g.name.clone())
            });
        let reply = self.run_player_request(verb, args);
        let (level, line) = arrivals::request_log(verb, name.as_deref(), args, &reply);
        super::log(level, &line);
        reply
    }

    /// Once a stop has begun (`/stop` from the console, a player or the control endpoint, or a stop
    /// signal), the transfers know it before the next request is answered: what would bring a player
    /// in is refused from then on, and the ways out stay open (`arrivals::Transfers::stopping`).
    pub(super) fn note_stopping(&mut self, stop: bool) {
        let stopping = stop || horizon_threads::stop_requests() > 0;
        if let Some(transfers) = self.lan.as_mut().and_then(|h| h.hub.transfers.as_mut()) {
            transfers.stopping |= stopping;
        }
    }

    /// Whether the tick loop ends now. A stop (`/stop`, a stop signal) ends it at once, unless
    /// `[transfer]` is on and players are here: the stop then waits for a proxy to move them out
    /// (`arrivals::StopWait`), every login refused, the server ticking on. A second `stop` ends the
    /// wait; a second signal ends the process (`horizon_threads::catch_stop_signals`).
    pub(super) fn stop_now(&mut self, state: &super::ServerState) -> bool {
        let asked = std::mem::take(&mut state.lists.lock().unwrap().stop);
        let stop = asked || horizon_threads::stop_requests() > 0;
        let Some(host) = self.lan.as_mut() else {
            return stop;
        };
        let waiting = host
            .hub
            .transfers
            .as_ref()
            .is_some_and(|t| t.stop_wait.is_some());
        if !stop && !waiting {
            return false;
        }
        let (placed, frozen) = arrivals::placed_and_frozen(&host.hub);
        let Some(transfers) = host.hub.transfers.as_mut() else {
            return stop;
        };
        let now = Instant::now();
        // A freeze lasts ticks: how long one takes now, for `check_freeze`.
        let period = arrivals::tick_period(self.clock.tick_duration(), state.tps());
        match &mut transfers.stop_wait {
            None => {
                transfers.stopping = true;
                let wait = arrivals::StopWait::start(&transfers.settings, now, period);
                let Some(wait) = wait.filter(|_| placed > 0) else {
                    return true;
                };
                transfers.stop_wait = Some(wait);
                host.hub.lan.shared.refuse_logins();
                super::log(
                    "INFO",
                    &arrivals::stop_wait_line(transfers.settings.stop_wait),
                );
                false
            }
            Some(_) if asked => {
                super::log("INFO", "A second stop: stopping now");
                true
            }
            Some(wait) => {
                wait.period = period;
                let over = wait.over(now, placed, frozen);
                if over && frozen > 0 {
                    super::log(
                        "WARN",
                        &format!(
                            "Stopping with {frozen} hand-over(s) still frozen: their players are saved here"
                        ),
                    );
                }
                over
            }
        }
    }

    /// `transfer-status`: the counts a proxy reads before it sends players here
    /// (`arrivals::status_request`). A proxy asks it every few seconds, so it writes nothing to
    /// the log.
    pub(super) fn transfer_status(&self) -> Value {
        match self.lan.as_ref() {
            Some(host) => arrivals::status_request(&host.hub, Instant::now()),
            None => json!({ "error": "the server is not open to players" }),
        }
    }

    fn run_player_request(&mut self, verb: &str, args: &str) -> Value {
        if matches!(
            verb,
            "player-profile-export" | "player-profile-status" | "player-profile-retire"
        ) {
            return self.run_profile_source_request(verb, args);
        }
        let Some(host) = self.lan.as_ref() else {
            return json!({ "error": "the server is not open to players" });
        };
        if host.hub.transfers.is_none() {
            return json!({ "error": arrivals::OFF });
        }
        match verb {
            "player-expect" => match self.lan.as_mut() {
                Some(host) => arrivals::expect_request(&mut host.hub, args, Instant::now()),
                None => json!({ "error": "the server is not open to players" }),
            },
            "player-profile-expect-return" => match self.lan.as_mut() {
                Some(host) => arrivals::expect_return_request(&mut host.hub, args, Instant::now()),
                None => json!({ "error": "the server is not open to players" }),
            },
            "player-profile-resume" => self.resume_profile_owner(args),
            "player-profile-prepare-paper" => self.prepare_paper_profile(args),
            "player-profile-import-paper" => self.stage_paper_profile(args),
            "player-profile-resume-paper" => self.resume_paper_profile(args),
            "player-drop" => match self.lan.as_mut() {
                Some(host) => arrivals::drop_request(&mut host.hub, args),
                None => json!({ "error": "the server is not open to players" }),
            },
            "player-export" => self.player_export(args),
            "player-import" => self.player_import(args),
            "player-confirm" => self.player_confirm(args),
            "player-release" => match self.lan.as_mut() {
                Some(host) => arrivals::release_request(&mut host.hub, args),
                None => json!({ "error": "the server is not open to players" }),
            },
            other => json!({ "error": format!("unknown request {other:?}") }),
        }
    }

    /// `player-export <uuid> [freeze]`: closes the player's menus as vanilla does when a menu closes
    /// (the cursor stack and the crafting grids go back into the inventory), then answers with the
    /// player's record (`arrivals::export_record`). With `freeze`, a hand-over starts: the player's
    /// own inputs stop being acted on until it ends, and a later export answers with the same record.
    fn player_export(&mut self, args: &str) -> Value {
        let mut parts = args.split_whitespace();
        let Some(uuid) = parts.next().and_then(arrivals::parse_uuid) else {
            return json!({ "error": "player-export needs the player's unique id" });
        };
        let freeze = match parts.next() {
            None => false,
            Some("freeze") => true,
            Some(_) => return json!({ "error": "the only option of player-export is freeze" }),
        };
        if parts.next().is_some() {
            return json!({ "error": "player-export takes a unique id and at most freeze" });
        }
        self.export_from(uuid, freeze)
    }

    /// `player-import <uuid> <json>`: applies a record to the held player (`lan/import.rs`), with
    /// its advancements and statistics, which need the game's tree and counters, and the mods'
    /// attachments. Unless the request says `"place": false`, it then places the player as
    /// `player-confirm` does, its file written first.
    fn player_import(&mut self, args: &str) -> Value {
        let Some((id, request)) = args.trim().split_once(char::is_whitespace) else {
            return json!({ "error": "player-import needs a unique id and a request" });
        };
        let Some(uuid) = arrivals::parse_uuid(id) else {
            return json!({ "error": "player-import needs the player's unique id" });
        };
        let request: Value = match serde_json::from_str(request.trim()) {
            Ok(v) => v,
            Err(e) => return json!({ "error": format!("the request is not JSON: {e}") }),
        };
        self.apply_player_import(uuid, request, false)
    }

    pub(super) fn apply_player_import(
        &mut self,
        uuid: u128,
        request: Value,
        projected_paper: bool,
    ) -> Value {
        let paper_digest = if projected_paper {
            let Some(digest) = request["record"]["paper_snapshot_sha256"]
                .as_str()
                .and_then(receipt::parse_digest)
            else {
                return json!({ "error": "the Paper projection has no snapshot digest" });
            };
            Some(digest)
        } else {
            None
        };
        let Some(host) = self.lan.as_ref() else {
            return json!({ "error": "the server is not open to players" });
        };
        let transfers = host.hub.transfers.as_ref().expect("transfers are enabled");
        let required = &transfers.settings.required_profiles;
        let strict = if transfers.settings.backend_id.is_some() {
            let binding = match receipt::Binding::from_import(&transfers.settings, uuid, &request) {
                Ok(binding) => binding,
                Err(error) => return json!({ "error": error }),
            };
            match receipt::lookup(&self.world_root, &binding) {
                Ok(Some(committed)) => {
                    let currently_placed =
                        host.hub.placed_arrival(uuid, binding.handover).is_some();
                    return committed.commit_reply(currently_placed, true);
                }
                Ok(None) => {}
                Err(error) => return json!({ "error": error }),
            }
            if host
                .hub
                .held_profile_return(uuid)
                .is_some_and(|authorization| !binding.matches_return(authorization))
            {
                return json!({ "error": "the profile import differs from the held return authorization" });
            }
            Some(binding)
        } else {
            None
        };
        if required.is_empty()
            && let Err(error) = import::check_record_profiles(&request["record"])
        {
            return json!({ "error": error });
        }
        // A retry after the commit, whose reply was lost: the player is placed from this
        // hand-over already, and the answer is the same.
        let retried = request["record"]["handover"]
            .as_str()
            .and_then(arrivals::parse_handover)
            .and_then(|id| host.hub.placed_arrival(uuid, id).map(|g| (id, g)));
        if let Some((id, i)) = retried {
            if strict.is_some() {
                return json!({ "error": "the placed hand-over has no matching durable arrival receipt" });
            }
            if !required.is_empty()
                && !import::is_profile_import_retry(&host.hub.guests[i], &request)
            {
                return json!({ "error": "the required profile request differs from the committed hand-over" });
            }
            return json!({
                "ok": true,
                "handover": arrivals::handover_text(id),
                "placed": true,
                "announced": host.hub.guests[i].arrival_announce,
                "recentered": host.hub.guests[i].arrival_recentered,
                "retried": true,
            });
        }
        let stage = if required.is_empty() {
            None
        } else {
            match import::stage_required_profiles(
                &host.hub,
                uuid,
                &request,
                required,
                |id, schema, payload, candidate| {
                    let mut reply = None;
                    self.native_player_transfer_profile_stage(
                        mod_api::__private::PlayerTransferProfileStageCall {
                            mod_id: id.namespace(),
                            owner: uuid,
                            id,
                            schema,
                            payload,
                            candidate,
                            reply: &mut reply,
                        },
                    );
                    reply.ok_or_else(|| {
                        format!("no approved receiver accepted required profile {id}")
                    })?
                },
            ) {
                Ok(stage) => Some(stage),
                Err(error) => return json!({ "error": error }),
            }
        };
        let hub = &mut self.lan.as_mut().expect("the server is open").hub;
        let result = match stage {
            Some(stage) if projected_paper => import::import_projected_paper_player(
                hub,
                &self.motion.attributes,
                uuid,
                &request,
                stage,
            ),
            Some(stage) => import::import_player_with_profiles(
                hub,
                &self.motion.attributes,
                uuid,
                &request,
                stage,
            ),
            None => import::import_player(hub, &self.motion.attributes, uuid, &request),
        };
        match result {
            Ok(done) => {
                let index = hub
                    .imported_arrival(uuid, done.handover)
                    .expect("the import just succeeded");
                let received = hub.guests.update(index, |guest| {
                    // Portable player data must not supply local authorities or receipts.
                    let _ = receipt::Hidden::take(guest);
                    strict
                        .as_ref()
                        .map(|binding| {
                            let mut received = receipt::Receipt::new(
                                binding.clone(),
                                done.announce,
                                done.recentered,
                            );
                            received.paper_snapshot_sha256 = paper_digest;
                            receipt::set_staged(guest, &received)
                        })
                        .transpose()
                });
                if let Err(error) = received {
                    return json!({ "error": error });
                }
                self.take_arrival_records(uuid, &done);
                self.sync_replaced_book_settings(uuid);
                let handover = arrivals::handover_text(done.handover);
                let placed = if done.place {
                    self.place_arrival(uuid, done.handover, None)
                } else {
                    Ok(false)
                };
                match placed {
                    Ok(placed) => {
                        let mut reply = json!({
                        "ok": true,
                        "handover": handover,
                        "parts": done.parts,
                        "placed": placed,
                        "announced": done.announce,
                        "recentered": done.recentered,
                        });
                        if let Some(binding) = strict {
                            binding.add_to_reply(&mut reply);
                        }
                        if let Some(digest) = paper_digest {
                            reply["paper_snapshot_sha256"] = json!(receipt::digest_hex(digest));
                        }
                        reply
                    }
                    Err(e) => json!({ "error": e }),
                }
            }
            Err(e) => json!({ "error": e }),
        }
    }

    /// What an import brought that the game keeps itself: the advancements (`PlayerAdvancements
    /// .applyFrom` drops those this server lacks) and the statistics.
    fn take_arrival_records(&mut self, uuid: u128, done: &horizon_client::lan::import::Imported) {
        let tree = self.advancement_tree();
        let host = self.lan.as_mut().expect("server host is present");
        let Some(index) = host.hub.guests.iter().position(|g| g.uuid == uuid) else {
            return;
        };
        host.hub.guests.update(index, |g| {
            if let Some(saved) = &done.advancements {
                let mut progress = horizon_client::advancements::Progress::from_json(saved);
                progress
                    .criteria
                    .retain(|id, _| tree.by_id.contains_key(id));
                g.advancements = progress;
                g.advancements_loaded = true;
            }
            if let Some(stats) = &done.stats {
                let play = host.play.entry(g.conn).or_default();
                play.stats = crate::game::stats::Stats::from_json(stats);
                play.stats.mark_all_dirty();
            }
        });
    }

    /// `player-confirm <uuid> <hand-over>`: places a held player whose record of that hand-over was
    /// imported without placing it.
    fn player_confirm(&mut self, args: &str) -> Value {
        let Some(host) = self.lan.as_ref() else {
            return json!({ "error": "the server is not open to players" });
        };
        if host
            .hub
            .transfers
            .as_ref()
            .is_some_and(|transfer| transfer.settings.backend_id.is_some())
        {
            return self.confirm_profile_arrival(args);
        }
        match arrivals::confirm_args(&host.hub, args) {
            // A retry after the commit, whose reply was lost: the same answer.
            Ok((uuid, id)) if host.hub.placed_arrival(uuid, id).is_some() => {
                let i = host.hub.placed_arrival(uuid, id).unwrap_or_default();
                json!({
                    "ok": true,
                    "placed": true,
                    "announced": host.hub.guests[i].arrival_announce,
                    "retried": true,
                })
            }
            Ok((uuid, id)) => match self.place_arrival(uuid, id, None) {
                Ok(_) => {
                    let announced = self
                        .lan
                        .as_ref()
                        .expect("server host is present")
                        .hub
                        .guests
                        .iter()
                        .find(|g| g.uuid == uuid)
                        .is_some_and(|g| g.arrival_announce);
                    json!({ "ok": true, "placed": true, "announced": announced })
                }
                Err(e) => json!({ "error": e }),
            },
            Err(e) => json!({ "error": e }),
        }
    }

    /// The authenticated proxy sends this after retiring the bound source record.
    fn confirm_profile_arrival(&mut self, args: &str) -> Value {
        let host = self.lan.as_ref().expect("the server is open");
        let transfers = host.hub.transfers.as_ref().expect("transfers are enabled");
        let binding = match receipt::Binding::from_confirm(&transfers.settings, args) {
            Ok(binding) => binding,
            Err(error) => return json!({ "error": error }),
        };
        let (owner, id) = (binding.owner, binding.handover);
        match receipt::lookup(&self.world_root, &binding) {
            Ok(Some(committed)) => {
                let returning = host
                    .hub
                    .held_profile_return(owner)
                    .filter(|authorization| binding.matches_return(authorization))
                    .filter(|_| host.hub.imported_arrival(owner, id).is_ok())
                    .cloned();
                let placed = host.hub.placed_arrival(owner, id).is_some();
                return match returning {
                    Some(authorization) if !transfers.stopping => {
                        match self.reconcile_return(&authorization) {
                            Ok(()) => committed.commit_reply(true, true),
                            Err(error) => json!({ "error": error }),
                        }
                    }
                    _ => committed.commit_reply(placed, true),
                };
            }
            Ok(None) => {}
            Err(error) => return json!({ "error": error }),
        }
        if transfers.stopping {
            return json!({ "error": arrivals::STOPPING });
        }
        let i = match host.hub.imported_arrival(owner, id) {
            Ok(i) => i,
            Err(error) => return json!({ "error": error }),
        };
        let staged = match receipt::staged(&host.hub.guests[i]) {
            Ok(Some(staged)) if staged.binding == binding => staged,
            Ok(_) => {
                return json!({ "error": "the strict hand-over differs from the held profile stage" });
            }
            Err(error) => return json!({ "error": error }),
        };
        if host
            .hub
            .held_profile_return(owner)
            .is_some_and(|authorization| !binding.matches_return(authorization))
        {
            return json!({ "error": "the confirmation differs from the held return authorization" });
        }
        match self.place_arrival(owner, id, Some(&staged)) {
            Ok(_) => staged.commit_reply(true, false),
            Err(error) => json!({ "error": error }),
        }
    }

    /// The hand-over's commit point on the target: the arrival's player file, advancements and
    /// statistics are written, synchronously, then it is placed. Only then does the reply say
    /// `ok`, so a proxy that sends `player-release moved` to the source after it knows a copy of
    /// the player is on disk here. When the file cannot be written the player stays held and
    /// imported: the proxy can confirm again or drop it.
    fn place_arrival(
        &mut self,
        uuid: u128,
        id: u128,
        receipt: Option<&receipt::Receipt>,
    ) -> Result<bool, String> {
        let hub = &self.lan.as_ref().expect("server host is present").hub;
        hub.imported_arrival(uuid, id)?;
        let returning = hub.held_profile_return(uuid).cloned();
        if let Some(authorization) = &returning {
            let journal = hub
                .lan
                .shared
                .config
                .source_handovers
                .as_ref()
                .ok_or("the source hand-over journal is unavailable")?;
            journal
                .validate_return(&authorization.destination, authorization)
                .map_err(|error| error.to_string())?;
        }
        // An autosave of this player still in flight must not land after this write.
        // Its completion fires save hooks, so the Host must still see the server.
        self.finish_player_autosaves(true)
            .map_err(|error| error.to_string())?;
        let host = self.lan.as_mut().expect("server host is present");
        let i = host.hub.imported_arrival(uuid, id)?;
        let mut g = host.hub.guests.take_for_save(i);
        let written = if playerdata::has_staged_arrival_profiles(&g) {
            // The current player.dat activates the profile and records the durable receipt.
            receipt
                .map(|receipt| {
                    receipt::append_receipt(&self.world_root, &mut g, receipt, returning.as_ref())
                })
                .transpose()
                .and_then(|previous| {
                    let result = self
                        .write_arrival_records(
                            self.lan.as_ref().expect("server host is present"),
                            &g,
                        )
                        .and_then(|()| {
                            playerdata::save_profile_arrival(&mut g, &self.world_root, id)
                        });
                    if result.is_err()
                        && let Some(previous) = previous
                    {
                        let _ = receipt::Hidden::take(&mut g);
                        previous.restore(&mut g);
                    }
                    result
                })
        } else if receipt.is_some() {
            Err("the strict hand-over has no prepared profile sections".into())
        } else {
            playerdata::save_now(&g, &self.world_root).and_then(|()| {
                self.write_arrival_records(self.lan.as_ref().expect("server host is present"), &g)
            })
        };
        let host = self.lan.as_mut().expect("server host is present");
        host.hub.guests.insert(i, g);
        written?;
        self.finish_arrival(uuid, id, returning.as_ref(), true)?;
        Ok(true)
    }

    fn reconcile_return(&mut self, authorization: &ReturnAuthorization) -> Result<(), String> {
        let (uuid, id) = (authorization.retired.owner, authorization.handover);
        let (current, tag) = CurrentReturn::read_matching_player(&self.world_root, authorization)
            .map_err(|error| error.to_string())?
            .ok_or("the current player file does not confirm this return")?;
        let host = self.lan.as_mut().expect("server host is present");
        let i = host.hub.imported_arrival(uuid, id)?;
        host.hub.guests.update(i, |guest| {
            playerdata::reconcile_profile_arrival(guest, current, tag)
        })?;
        self.finish_arrival(uuid, id, Some(authorization), true)
    }

    /// A current SQL owner can resume an already committed return. The authenticated
    /// coordinator checks the exact owner token, backend and revision before requesting this.
    fn resume_profile_owner(&mut self, args: &str) -> Value {
        let reply = (|| {
            let (owner, request) = args
                .trim()
                .split_once(char::is_whitespace)
                .ok_or("player-profile-resume needs the player's unique id and authorization")?;
            let owner = arrivals::parse_uuid(owner).ok_or("invalid player unique id")?;
            let authorization: ReturnAuthorization = serde_json::from_str(request.trim())
                .map_err(|_| "invalid profile resume authorization")?;
            if authorization.retired.owner != owner {
                return Err("the resume authorization belongs to a different owner".into());
            }
            let retried = self.resume_committed_profile(&authorization)?;
            Ok(json!({
                "ok": true,
                "placed": true,
                "currently_placed": true,
                "handover": arrivals::handover_text(authorization.handover),
                "authority_revision": u64::from(authorization.revision),
                "retried": retried,
            }))
        })();
        reply.unwrap_or_else(|error: String| json!({ "error": error }))
    }

    fn resume_committed_profile(
        &mut self,
        authorization: &ReturnAuthorization,
    ) -> Result<bool, String> {
        let (owner, id) = (authorization.retired.owner, authorization.handover);
        let hub = &self.lan.as_ref().expect("the server is open").hub;
        let transfers = hub.transfers.as_ref().expect("transfers are enabled");
        if transfers.stopping {
            return Err(arrivals::STOPPING.into());
        }
        if hub.placed_arrival(owner, id).is_some_and(|i| {
            let guest = &hub.guests[i];
            guest.participates_in_player_simulation() && guest.left_reason.is_empty()
        }) && hub
            .lan
            .shared
            .config
            .source_handovers
            .as_ref()
            .is_some_and(|journal| journal.active_return_matches(authorization))
        {
            return Ok(true);
        }
        if hub.held_profile_return(owner) != Some(authorization) {
            return Err("no forwarded held player matches the current-owner authorization".into());
        }
        self.finish_player_autosaves(true)
            .map_err(|error| error.to_string())?;
        let (_, tag) = CurrentReturn::read_matching_player(&self.world_root, authorization)
            .map_err(|error| error.to_string())?
            .ok_or("the current player file does not confirm this return")?;
        self.verify_committed_profiles(owner, id, &tag)?;
        self.restore_held_profile(owner, id, tag)?;
        self.finish_arrival(owner, id, Some(authorization), false)?;
        Ok(false)
    }

    fn verify_committed_profiles(
        &self,
        owner: u128,
        id: u128,
        tag: &horizon_core::nbt::Tag,
    ) -> Result<(), String> {
        let hub = &self.lan.as_ref().expect("the server is open").hub;
        for (profile, &schema) in &hub
            .transfers
            .as_ref()
            .expect("transfers are enabled")
            .settings
            .required_profiles
        {
            playerdata::verify_committed_profile(
                &self.world_root,
                owner,
                id,
                tag,
                profile,
                schema,
            )?;
        }
        Ok(())
    }

    fn restore_held_profile(
        &mut self,
        owner: u128,
        id: u128,
        tag: horizon_core::nbt::Tag,
    ) -> Result<(), String> {
        let hub = &mut self.lan.as_mut().expect("the server is open").hub;
        let mut level = playerdata::saved_player(tag);
        if !hub.config.levels.has(&level.dimension) {
            level.dimension = "minecraft:overworld".into();
        }
        let i = hub
            .guests
            .iter()
            .position(|g| g.uuid == owner && g.held)
            .ok_or("the held connection went away")?;
        let forced_mode = hub
            .config
            .force_game_mode
            .then_some(hub.config.default_game_mode);
        hub.guests.update(i, |guest| {
            playerdata::resume_profile_arrival(guest, owner, id, &level)?;
            if let Some(mode) = forced_mode {
                guest.game_mode = mode;
                guest.abilities.update_for_mode(mode);
            }
            Ok::<_, String>(())
        })?;
        let conn = hub.guests[i].conn;
        self.load_guest_records(conn);
        Ok(())
    }

    fn finish_arrival(
        &mut self,
        uuid: u128,
        id: u128,
        returning: Option<&ReturnAuthorization>,
        saved: bool,
    ) -> Result<(), String> {
        let host = self.lan.as_mut().expect("server host is present");
        let i = host.hub.imported_arrival(uuid, id)?;
        let guest = &host.hub.guests[i];
        if playerdata::has_staged_arrival_profiles(guest) {
            return Err("the required profile sections have not been saved".into());
        }
        let (conn, name) = (guest.conn, guest.name.clone());
        if let Some(authorization) = returning {
            host.hub
                .lan
                .shared
                .config
                .source_handovers
                .as_ref()
                .ok_or("the source hand-over journal is unavailable")?
                .activate_return(&authorization.destination, authorization)
                .map_err(|error| error.to_string())?;
        }
        if !saved {
            host.hub.send_arrival_state(i);
        }
        let announced = host.hub.confirm_arrival(uuid, id)?;
        if !saved {
            host.hub.send_post_effects(i);
            self.sync_replaced_book_settings(uuid);
        }
        let directory = playerdata::directory(&self.world_root);
        if returning.is_some() || !saved {
            self.bind_player_attachments(conn, false);
        }
        if returning.is_some() {
            self.native_player_load_from_file(uuid, &name, &directory);
        }
        if saved {
            self.native_player_save_to_file(uuid, &name, &directory);
        }
        self.arrival_placed(uuid, &name, announced);
        Ok(())
    }

    /// What the game does once a held player is placed: the log's lines, and the mods' "logged
    /// in" (`native_player_logged_in`), which a held login did not send, so a placed arrival
    /// is heard of as any login and its leaving later is not the mods' first word of it.
    fn arrival_placed(&mut self, uuid: u128, name: &str, announced: bool) {
        super::log("INFO", &arrivals::placed_line(name));
        if announced {
            super::log("INFO", &format!("System chat: {name} joined the game"));
        }
        self.announce_login(uuid, name, false);
    }

    fn export_from(&mut self, uuid: u128, freeze: bool) -> Value {
        let host = self.lan.as_ref().expect("server host is present");
        let i = match arrivals::check_exportable(&host.hub, uuid) {
            Ok(i) => i,
            Err(why) => return json!({ "error": why }),
        };
        if host.hub.guests[i].is_fake() {
            return json!({ "error": "fake players cannot be exported" });
        }
        if freeze && let Err(why) = arrivals::check_freeze(&host.hub, i, Instant::now()) {
            return json!({ "error": why });
        }
        // A repeat while the hand-over lasts: the same record, and nothing touched.
        if let Some(h) = &host.hub.guests[i].handover {
            return arrivals::export_reply(&host.hub, i, h.record.clone());
        }
        let conn = host.hub.guests[i].conn;
        let i = match self.settle_transfer_menus(conn) {
            Ok(i) => i,
            Err(error) => return json!({ "error": error }),
        };
        let records = self.transfer_source_records(i);
        let host = self.lan.as_mut().expect("server host is present");
        let local_receipts = host.hub.guests.update(i, receipt::Hidden::take);
        let exported = arrivals::export_record(&mut host.hub, i, freeze, records);
        host.hub
            .guests
            .update(i, |guest| local_receipts.restore(guest));
        match exported {
            Ok(record) => arrivals::export_reply(&host.hub, i, record),
            Err(e) => json!({ "error": e }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::crypto::uuid_string;

    #[test]
    fn versioned_receipt_binds_exact_return_in_current_player_file() {
        use horizon_client::lan::{
            Joined, PlayerSender, guest::Guest, source_handover::RetiredAnchor,
        };
        use horizon_core::identifier::Identifier;
        use horizon_protocol::packets::configuration::ClientInformation;
        use std::sync::{Arc, mpsc};

        let world = tempfile::tempdir().unwrap();
        let owner = 12;
        let id = 51;
        let settings = arrivals::Settings {
            backend_id: Some("west".into()),
            allowed_sources: ["east".into()].into(),
            required_profiles: [(Identifier::parse_namespaced("slimefun:profile").unwrap(), 1)]
                .into(),
            ..arrivals::Settings::default()
        };
        let request = json!({
            "record": {
                "version": 2,
                "uuid": uuid_string(owner),
                "handover": arrivals::handover_text(id),
                "source": "east",
                "destination": "west",
                "authority_revision": 9,
            },
            "place": false,
        });
        let binding = receipt::Binding::from_import(&settings, owner, &request).unwrap();
        let authorization = ReturnAuthorization {
            retired: RetiredAnchor {
                owner,
                handover: 49,
                revision: 7.try_into().unwrap(),
                record_sha256: [11; 32],
            },
            handover: id,
            revision: 9.try_into().unwrap(),
            source: "east".into(),
            destination: "west".into(),
            record_sha256: receipt::record_digest(&request["record"]).unwrap(),
            request_sha256: receipt::record_digest(&request).unwrap(),
        };
        let confirm = format!(
            "{} {} east west {} {}",
            uuid_string(owner),
            arrivals::handover_text(id),
            receipt::digest_hex(authorization.record_sha256),
            receipt::digest_hex(authorization.request_sha256),
        );
        assert_ne!(
            receipt::Binding::from_confirm(&settings, &confirm).unwrap(),
            binding
        );
        assert_eq!(
            receipt::Binding::from_confirm(&settings, &format!("{confirm} 9")).unwrap(),
            binding
        );
        assert!(receipt::Binding::from_confirm(&settings, &format!("{confirm} 0")).is_err());
        let received = receipt::Receipt::new(binding, false, false);
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn: 12,
                uuid: owner,
                name: "guest12".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::default(),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: true,
            },
            12,
            [3.0, 64.0, 5.0],
            10,
        );
        let mut wrong = authorization.clone();
        wrong.request_sha256 = [0; 32];
        assert!(
            receipt::append_receipt(world.path(), &mut guest, &received, Some(&wrong)).is_err()
        );
        assert!(!playerdata::path(world.path(), owner).exists());
        receipt::append_receipt(world.path(), &mut guest, &received, Some(&authorization)).unwrap();
        playerdata::save_now(&guest, world.path()).unwrap();
        let current = CurrentReturn::read(world.path(), owner).unwrap().unwrap();
        assert_eq!(current.authorization(), &authorization);
        assert_eq!(
            receipt::lookup(world.path(), &received.binding).unwrap(),
            Some(received)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn strict_arrival_receipts_survive_reopening_and_a_new_handover() {
        use horizon_client::lan::playerdata;
        use horizon_core::{identifier::Identifier, nbt::Tag};

        let world = tempfile::tempdir().unwrap();
        let uuid = uuid_string(12);
        let file = playerdata::path(world.path(), 12);
        let mut original = None;
        let strict_settings = || arrivals::Settings {
            backend_id: Some("beta".into()),
            allowed_sources: ["alpha".into(), "gamma".into()].into(),
            required_profiles: [(Identifier::parse_namespaced("slimefun:profile").unwrap(), 1)]
                .into(),
            ..arrivals::Settings::default()
        };
        let stale = || {
            Tag::compound([(
                "PlayerPersisted",
                Tag::compound([("slimefun:stale", Tag::Byte(1))]),
            )])
        };
        let stage = |game: &mut Game<'_>, request: &Value| {
            let mods = vec![mod_api::ModEntry::new(
                "slimefun",
                Box::new(ProfileReceiver(true)),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                game.player_request("player-import", &format!("{uuid} {request}"))
            })
        };
        let confirm_args = |stage: &Value| {
            let names = [
                "handover",
                "source",
                "destination",
                "record_sha256",
                "request_sha256",
            ];
            let mut args = uuid.clone();
            for name in names {
                args.push(' ');
                args.push_str(
                    stage[name]
                        .as_str()
                        .expect("successful strict stage supplies its tuple"),
                );
            }
            args
        };
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.world_root = world.path().to_owned();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let index = hub.guests.connection_index(12).unwrap();
            hub.transfers = Some(arrivals::Transfers::new(arrivals::Settings::default()));
            let mut record = arrivals::export_record(hub, index, true, Default::default()).unwrap();
            hub.transfers = Some(arrivals::Transfers::new(strict_settings()));
            hub.guests.update(index, |guest| {
                guest.handover = None;
                guest.placed = false;
                guest.held = true;
                guest.saved = Some(Tag::compound([("NeoForgeData", stale())]));
            });
            record["version"] = json!(2);
            record["source"] = json!("alpha");
            record["destination"] = json!("beta");
            record["profiles"] =
                json!([{"id":"slimefun:profile","version":1,"payload":"U0ZQMQ=="}]);
            let request = json!({"record": record, "place": false, "parts": ["experience"]});
            let mut invalid = request.clone();
            invalid["place"] = json!(true);
            assert!(
                stage(game, &invalid)["error"]
                    .as_str()
                    .unwrap()
                    .contains("place: false")
            );
            invalid = request.clone();
            invalid["record"]["source"] = json!("untrusted");
            assert!(
                stage(game, &invalid)["error"]
                    .as_str()
                    .unwrap()
                    .contains("route is not approved")
            );
            let accepted = stage(game, &request);
            assert_eq!(accepted["placed"], false, "{accepted}");
            assert!(!file.exists());
            let confirm = confirm_args(&accepted);
            let wrong = format!("{}{}", &confirm[..confirm.len() - 64], "0".repeat(64));
            assert!(
                game.player_request("player-confirm", &wrong)["error"]
                    .as_str()
                    .unwrap()
                    .contains("held profile stage")
            );
            let bare = format!("{uuid} {}", accepted["handover"].as_str().unwrap());
            assert!(game.player_request("player-confirm", &bare)["error"].is_string());
            assert!(!file.exists());
            let placed = game.player_request("player-confirm", &confirm);
            assert_eq!(placed["placed"], true, "{placed}");
            assert_eq!(placed["currently_placed"], true);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.update(index, |guest| guest.xp_level = 41);
            playerdata::save_now(&hub.guests[index], world.path()).unwrap();
            original = Some((request, confirm));
        });
        let (request, confirm) = original.unwrap();
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.world_root = world.path().to_owned();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let index = hub.guests.connection_index(12).unwrap();
            hub.transfers = Some(arrivals::Transfers::new(strict_settings()));
            hub.guests.update(index, |guest| {
                assert!(playerdata::load(guest, world.path()));
                guest.held = true;
                guest.placed = false;
            });
            let recovered = std::fs::read(&file).unwrap();
            assert!(file.with_extension("dat_old").is_file());
            std::fs::write(&file, b"corrupt current arrival file").unwrap();
            let corrupt = game.player_request("player-confirm", &confirm);
            assert!(
                corrupt["error"]
                    .as_str()
                    .unwrap()
                    .contains("player file is corrupt"),
                "{corrupt}"
            );
            std::fs::write(&file, &recovered).unwrap();
            for (verb, args) in [
                ("player-import", format!("{uuid} {request}")),
                ("player-confirm", confirm.clone()),
            ] {
                let historical = game.player_request(verb, &args);
                assert_eq!(historical["already_committed"], true, "{historical}");
                assert_eq!(historical["currently_placed"], false);
            }
            for (field, replacement) in [
                ("source", json!("gamma")),
                (
                    "profiles",
                    json!([{"id":"slimefun:profile","version":1,"payload":"U0ZQMg=="}]),
                ),
            ] {
                let mut substituted = request.clone();
                substituted["record"][field] = replacement;
                let refused =
                    game.player_request("player-import", &format!("{uuid} {substituted}"));
                assert!(
                    refused["error"]
                        .as_str()
                        .unwrap()
                        .contains("durable arrival receipt"),
                    "{refused}"
                );
            }
            let mut changed_options = request.clone();
            changed_options["announce"] = json!(false);
            assert!(
                game.player_request("player-import", &format!("{uuid} {changed_options}"))["error"]
                    .is_string()
            );
            assert_eq!(std::fs::read(&file).unwrap(), recovered);
            let guest = &game.lan.as_ref().unwrap().hub.guests[index];
            assert!(guest.held && !guest.imported && !guest.placed);
            assert_eq!(guest.xp_level, 41);

            let mut next = request.clone();
            next["record"]["handover"] = json!("00000000000000000000000000000004");
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .guests
                .update(index, |guest| {
                    // This is a new stage, with the owner's current package data.
                    let Some(Tag::Compound(saved)) = guest.saved.as_mut() else {
                        panic!("loaded player data");
                    };
                    saved.insert("NeoForgeData".into(), stale());
                });
            let next = stage(game, &next);
            assert_eq!(next["placed"], false, "{next}");
            let next_confirmation = game.player_request("player-confirm", &confirm_args(&next));
            assert_eq!(next_confirmation["placed"], true, "{next_confirmation}");
            let after = std::fs::read(&file).unwrap();
            let historical = game.player_request("player-confirm", &confirm);
            assert_eq!(historical["already_committed"], true, "{historical}");
            assert_eq!(historical["currently_placed"], false);
            assert_eq!(std::fs::read(&file).unwrap(), after);
        });
    }

    #[derive(Default)]
    struct ProfileReceiver(bool);

    #[mod_api::horizon_mod]
    impl mod_api::Hooks for ProfileReceiver {
        fn player_transfer_profile_stage(
            &self,
            host: &dyn mod_api::ServerHost,
            event: &mut mod_api::PlayerTransferProfileStageEvent<'_>,
        ) {
            assert!(self.0, "the foreign package received private profile bytes");
            assert_eq!(event.mod_id, "slimefun");
            assert_eq!(event.owner, 12);
            assert_eq!(event.id.to_string(), "slimefun:profile");
            assert_eq!(event.schema, 1);
            assert_eq!(event.payload, b"SFP1");
            let flags = event.candidate.get(&"PlayerPersisted".into()).unwrap();
            assert_eq!(
                flags.get("slimefun:stale"),
                Some(&horizon_core::nbt::Tag::Byte(1))
            );
            assert!(flags.get("other:keep").is_none());
            assert!(host.item_from_saved_gzip_nbt(Vec::new()).is_none());
            *event.reply = Some(Ok([(
                "slimefun:issued".into(),
                horizon_core::nbt::Tag::Byte(1),
            )]
            .into()));
        }
    }

    #[test]
    #[ignore = "daily"]
    fn required_profiles_neither_stage_nor_acknowledge_a_profile_free_commit() {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let world = tempfile::tempdir().unwrap();
            game.world_root = world.path().to_owned();
            let owner = 12;
            let (index, record) = {
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(arrivals::Transfers::new(arrivals::Settings::default()));
                let index = hub.guests.connection_index(12).unwrap();
                hub.guests.update(index, |guest| guest.xp_level = 7);
                let record =
                    arrivals::export_record(hub, index, true, arrivals::Records::default())
                        .unwrap();
                hub.guests.update(index, |guest| {
                    guest.handover = None;
                    guest.placed = false;
                    guest.held = true;
                    guest.xp_level = 0;
                });
                (index, record)
            };
            let file = playerdata::path(world.path(), owner);
            let handover = record["handover"].as_str().unwrap();
            let uuid = uuid_string(owner);
            let stock = json!({"record": record, "place": false, "parts": ["experience"]});
            let mut profiled = stock.clone();
            profiled["record"]["version"] = json!(2);
            profiled["record"]["profiles"] = json!([{
                "id": "slimefun:profile", "version": 1, "payload": "U0ZQMQ=="
            }]);
            let unsupported = |reply: &Value| {
                assert!(
                    reply["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("no durable receiver")),
                    "{reply}"
                );
            };

            unsupported(&game.player_request("player-import", &format!("{uuid} {profiled}")));
            assert!(!file.exists());
            let guest = &game.lan.as_ref().unwrap().hub.guests[index];
            assert!(guest.held && !guest.placed && !guest.imported);
            assert_eq!(guest.xp_level, 0);
            assert!(
                game.player_request("player-confirm", &format!("{uuid} {handover}"))["error"]
                    .is_string()
            );
            assert!(!file.exists());

            let staged = game.player_request("player-import", &format!("{uuid} {stock}"));
            assert_eq!(staged["ok"], true, "{staged}");
            assert_eq!(staged["placed"], false);
            assert!(!file.exists());
            let placed = game.player_request("player-confirm", &format!("{uuid} {handover}"));
            assert_eq!(placed["ok"], true, "{placed}");
            assert_eq!(placed["placed"], true);
            let saved = std::fs::read(&file).unwrap();
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[index].xp_level, 7);
            assert_eq!(
                game.player_request("player-import", &format!("{uuid} {stock}"))["retried"],
                true
            );
            unsupported(&game.player_request("player-import", &format!("{uuid} {profiled}")));
            assert_eq!(std::fs::read(&file).unwrap(), saved);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn staged_profiles_activate_only_with_the_durable_player_file() {
        use horizon_client::lan::{import, playerdata};
        use horizon_client::save::read_dat;
        use horizon_core::{identifier::Identifier, nbt::Tag};
        use mod_api::{
            ServerHost, TransferProfileReadError as Error,
            content::crash::{ModCall, enter_native_mod},
        };

        crate::game::native_guest_turn_tests::with_guests(|game| {
            let world = tempfile::tempdir().unwrap();
            game.world_root = world.path().to_owned();
            let owner = 12;
            let (index, mut record) = {
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.transfers = Some(arrivals::Transfers::new(arrivals::Settings::default()));
                let index = hub.guests.connection_index(12).unwrap();
                let record = arrivals::export_record(hub, index, true, Default::default()).unwrap();
                hub.guests.update(index, |guest| {
                    guest.handover = None;
                    guest.placed = false;
                    guest.held = true;
                    guest.saved = Some(Tag::compound([(
                        "NeoForgeData",
                        Tag::compound([(
                            "PlayerPersisted",
                            Tag::compound([
                                ("slimefun:stale", Tag::Byte(1)),
                                ("other:keep", Tag::Byte(1)),
                            ]),
                        )]),
                    )]));
                });
                playerdata::save_now(&hub.guests[index], world.path()).unwrap();
                (index, record)
            };
            let handover = record["handover"].as_str().unwrap().to_owned();
            let uuid = uuid_string(owner);
            let id = Identifier::parse_namespaced("slimefun:profile").unwrap();
            let foreign = Identifier::parse_namespaced("other:profile").unwrap();
            let host: &dyn ServerHost = game;
            assert_eq!(
                host.player_transfer_profile_generation(owner, &id),
                Err(Error::NotOwned)
            );
            let _scope = enter_native_mod(ModCall {
                id: "slimefun",
                hook: "test_transfer_profile_read",
                part: None,
            });
            assert_eq!(
                host.player_transfer_profile_generation(owner, &foreign),
                Err(Error::NotOwned)
            );
            assert_eq!(
                host.player_transfer_profile_generation(owner, &id),
                Err(Error::Transferring)
            );
            assert_eq!(host.player_transfer_profile_generation(999, &id), Ok(None));
            record["version"] = json!(2);
            record["profiles"] = json!([{
                "id": id.to_string(), "version": 1, "payload": "U0ZQMQ=="
            }]);
            let request = json!({"record": record, "place": false, "parts": ["experience"]});
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let roster = [(id.clone(), 1)].into();
            let check = import::stage_required_profiles(
                hub,
                owner,
                &request,
                &roster,
                |actual_id, version, payload, data| {
                    assert_eq!((actual_id, version, payload), (&id, 1, &b"SFP1"[..]));
                    assert_eq!(
                        data.get(&"PlayerPersisted".into())
                            .and_then(|tag| tag.get("other:keep")),
                        None
                    );
                    Ok([("other:overwrite".into(), Tag::Byte(1))].into())
                },
            );
            assert!(check.err().unwrap().contains("another mod"));
            hub.transfers.as_mut().unwrap().settings.required_profiles = roster;
            let mut stock = request.clone();
            stock["record"]["version"] = json!(1);
            stock["record"].as_object_mut().unwrap().remove("profiles");
            assert!(
                game.player_request("player-import", &format!("{uuid} {stock}"))["error"]
                    .is_string()
            );
            let other = || mod_api::ModEntry::new("other", Box::new(ProfileReceiver(false)));
            let missing = mod_platform::testing::with_mods(vec![other()].into(), || {
                game.player_request("player-import", &format!("{uuid} {request}"))
            });
            assert!(
                missing["error"]
                    .as_str()
                    .unwrap()
                    .contains("no approved receiver")
            );
            let mods = vec![
                other(),
                mod_api::ModEntry::new("slimefun", Box::new(ProfileReceiver(true))),
            ]
            .into();
            let staged = mod_platform::testing::with_mods(mods, || {
                game.player_request("player-import", &format!("{uuid} {request}"))
            });
            assert_eq!(staged["placed"], false, "{staged}");
            let hold = arrivals::parse_handover(&handover).unwrap();
            assert!(
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .confirm_arrival(owner, hold)
                    .is_err()
            );

            let file = playerdata::path(world.path(), owner);
            let backup = file.with_extension("dat_old");
            let before = std::fs::read(&file).unwrap();
            std::fs::create_dir(&backup).unwrap();
            let refused = game.player_request("player-confirm", &format!("{uuid} {handover}"));
            assert!(refused["error"].is_string(), "{refused}");
            assert_eq!(std::fs::read(&file).unwrap(), before);
            let guest = &game.lan.as_ref().unwrap().hub.guests[index];
            assert!(guest.held && !guest.placed);
            let section_files = std::fs::read_dir(file.parent().unwrap())
                .unwrap()
                .flatten()
                .map(|entry| entry.path())
                .filter(|file| file.extension().is_some_and(|ext| ext == "hprofile"))
                .collect::<Vec<_>>();
            assert_eq!(section_files.len(), 1);
            assert_eq!(std::fs::read(&section_files[0]).unwrap(), b"SFP1");

            std::fs::remove_dir(&backup).unwrap();
            let confirmed = game.player_request("player-confirm", &format!("{uuid} {handover}"));
            assert_eq!(confirmed["placed"], true, "{confirmed}");
            assert_eq!(
                game.player_request("player-import", &format!("{uuid} {request}"))["retried"],
                true
            );
            assert!(
                game.player_request("player-import", &format!("{uuid} {stock}"))["error"]
                    .is_string()
            );
            let mut changed = request.clone();
            changed["record"]["profiles"][0]["payload"] = json!("U0ZQMg==");
            assert!(
                game.player_request("player-import", &format!("{uuid} {changed}"))["error"]
                    .is_string()
            );
            let saved = read_dat(&file).unwrap();
            let active = saved
                .get("HorizonTransferProfiles")
                .and_then(|tag| tag.get("slimefun:profile"));
            assert_eq!(active.and_then(|tag| tag.get("length")), Some(&Tag::Int(4)));
            let persisted = saved
                .get("NeoForgeData")
                .unwrap()
                .get("PlayerPersisted")
                .unwrap();
            assert_eq!(persisted.get("slimefun:issued"), Some(&Tag::Byte(1)));
            assert_eq!(persisted.get("other:keep"), Some(&Tag::Byte(1)));
            assert!(persisted.get("slimefun:stale").is_none());
            let refused = game.player_request("player-export", &format!("{uuid} freeze"));
            assert!(
                refused["error"]
                    .as_str()
                    .unwrap()
                    .contains("no approved source profile collector")
            );
            let generation = arrivals::parse_handover(&handover).unwrap();
            let verify_owned = |host: &dyn ServerHost| {
                assert_eq!(
                    host.player_transfer_profile_generation(owner, &id),
                    Ok(Some(generation))
                );
                assert_eq!(
                    host.player_transfer_profile_payload(owner, &id, generation),
                    Ok(b"SFP1".to_vec())
                );
                assert_eq!(
                    host.player_transfer_profile_payload(owner, &id, generation ^ 1),
                    Err(Error::StaleGeneration)
                );
            };
            verify_owned(game);
            let hub = &mut game.lan.as_mut().unwrap().hub;
            let leaving = hub.guests.connection_index(13).unwrap();
            hub.guests.update(leaving, |guest| guest.released = true);
            let host: &dyn ServerHost = game;
            assert_eq!(
                host.player_transfer_profile_generation(13, &id),
                Err(Error::Away)
            );
            game.lan.as_mut().unwrap().hub.guests.remove(index);
            verify_owned(game);

            let player_file = std::fs::read(&file).unwrap();
            std::fs::write(&file, b"damaged current player file").unwrap();
            let host: &dyn ServerHost = game;
            assert!(backup.is_file(), "the unmarked backup exists");
            assert_eq!(
                host.player_transfer_profile_generation(owner, &id),
                Err(Error::Corrupt)
            );
            std::fs::write(&file, player_file).unwrap();
            std::fs::write(&section_files[0], b"oops").unwrap();
            assert_eq!(
                host.player_transfer_profile_payload(owner, &id, generation),
                Err(Error::Corrupt)
            );
            std::fs::remove_file(&section_files[0]).unwrap();
            assert_eq!(
                host.player_transfer_profile_generation(owner, &id),
                Err(Error::Corrupt)
            );
        });
    }
}
