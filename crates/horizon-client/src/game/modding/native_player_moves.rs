//! `player_move`: the game's half of a movement report the hub holds for the mods.
use super::{Dim, Game};
use horizon_client::lan::hub::Hub;

impl Game<'_> {
    /// What the hub needs to know of the loaded mods each tick.
    pub(in crate::game) fn sync_hub_mod_flags(hub: &mut Hub) {
        hub.mods_loaded = !mod_api::__private::mods().is_empty();
        hub.moves_listened = mod_api::__private::listened(mod_api::HookId::PlayerMove)
            || mod_api::__private::listened(mod_api::HookId::PlayerPositionCommitted);
        hub.sprint_attempts_listened =
            mod_api::__private::listened(mod_api::HookId::PlayerSprintPre);
    }

    /// The hub holds guest `conn`'s movement report: the mods hear it with the player's state
    /// swapped in and nothing of the hub borrowed, then the hub commits it or drops it and
    /// corrects the client. A report the hub no longer holds (the player left or was teleported
    /// first) is not heard.
    pub(in crate::game) fn lan_player_move(&mut self, conn: u32) {
        let Some(report) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.pending_move_report(conn))
        else {
            return;
        };
        let mut cancel = false;
        if mod_api::__private::listened(mod_api::HookId::PlayerMove) {
            self.as_guest(conn, |game| {
                game.native_player_move(
                    report.player,
                    report.from,
                    report.from_rotation,
                    report.to,
                    report.to_rotation,
                    &mut cancel,
                );
            });
        }
        let Some(from) = self
            .lan
            .as_mut()
            .and_then(|host| host.hub.resolve_move(conn, !cancel))
        else {
            return;
        };
        if self.player_position_listened() && from != report.to {
            self.as_guest(conn, |game| {
                game.notify_player_position(report.player, (game.dim, from), (game.dim, report.to));
            });
        }
    }

    pub(in crate::game) fn player_position_listened(&self) -> bool {
        self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::PlayerPositionCommitted)
    }

    pub(in crate::game) fn active_teleport_origin(&self) -> Option<(u128, Dim, [f64; 3])> {
        if !self.player_position_listened() {
            return None;
        }
        let uuid = self.native_acting_player()?.uuid();
        let at = self.player.as_ref()?.position;
        Some((uuid, self.dim, [at.x, at.y, at.z]))
    }

    pub(in crate::game) fn notify_active_teleport(
        &mut self,
        origin: Option<(u128, Dim, [f64; 3])>,
    ) {
        let (Some((uuid, dim, from)), Some(player)) = (origin, self.player.as_ref()) else {
            return;
        };
        let at = player.position;
        self.notify_player_position(uuid, (dim, from), (self.dim, [at.x, at.y, at.z]));
    }

    pub(in crate::game) fn notify_player_position(
        &mut self,
        player: u128,
        origin: (Dim, [f64; 3]),
        destination: (Dim, [f64; 3]),
    ) {
        if origin != destination && self.player_position_listened() {
            self.native_player_position_committed(
                player,
                origin.0.id(),
                origin.1,
                destination.0.id(),
                destination.1,
            );
        }
    }
}
