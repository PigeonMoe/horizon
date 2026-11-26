use super::*;
use crate::game::Dim;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(in crate::game) fn native_player_position_committed(
        &mut self,
        player: u128,
        from_level: &str,
        from: [f64; 3],
        to_level: &str,
        to: [f64; 3],
    ) {
        self.fire_player_position_committed(player, from_level, from, to_level, to);
    }

    pub(in crate::game) fn native_player_teleport_pre(
        &mut self,
        call: mod_api::__private::PlayerTeleportPreCall<'_>,
    ) {
        self.fire_player_teleport_pre(call);
    }

    pub(in crate::game) fn player_teleport_allowed(
        &mut self,
        player: u128,
        origin: (Dim, [f64; 3]),
        destination: (Dim, [f64; 3]),
        cause: &str,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::PlayerTeleportPre)
        {
            return true;
        }
        let mut cancel = false;
        self.native_player_teleport_pre(mod_api::__private::PlayerTeleportPreCall {
            player,
            from_level: origin.0.id(),
            from: origin.1,
            to_level: destination.0.id(),
            to: destination.1,
            cause,
            cancel: &mut cancel,
        });
        !cancel
    }

    /// Teleport the player whose body is in place. A callback may move or
    /// replace that body, so a later commit must still belong to its source.
    pub(in crate::game) fn current_player_teleport_allowed(
        &mut self,
        target: Vec3,
        cause: &str,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::PlayerTeleportPre)
        {
            return true;
        }
        let Some(player) = self.player.as_ref() else {
            return false;
        };
        let id = player.entity_id;
        let from = player.position;
        let uuid = self.acting_uuid();
        let level = self.dim;
        self.player_teleport_allowed(
            horizon_client::lan::playerdata::ints_uuid(uuid),
            (level, [from.x, from.y, from.z]),
            (level, [target.x, target.y, target.z]),
            cause,
        ) && self.dim == level
            && self.acting_uuid() == uuid
            && self.player.as_ref().is_some_and(|player| {
                player.entity_id == id && player.position == from && !player.health.dead_or_dying()
            })
    }
}
