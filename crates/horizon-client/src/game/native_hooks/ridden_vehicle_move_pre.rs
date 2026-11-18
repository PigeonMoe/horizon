use super::*;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(in crate::game) fn native_ridden_vehicle_move_pre(
        &self,
        player: u128,
        vehicle: u32,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_ridden_vehicle_move_pre(player, vehicle, dimension, from, to, cancel);
    }

    pub(in crate::game) fn ridden_vehicle_move_allowed(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
    ) -> bool {
        self.ridden_vehicle_move_allowed_with(vehicle, from, to, |seat| {
            self.vehicle_carries_seat(vehicle, seat)
        })
    }

    pub(in crate::game) fn ridden_vehicle_move_allowed_with(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
        carried: impl Fn(u32) -> bool,
    ) -> bool {
        if from == to
            || !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::RiddenVehicleMovePre)
        {
            return true;
        }
        let allows = |uuid| {
            let mut cancel = false;
            self.native_ridden_vehicle_move_pre(
                uuid,
                vehicle,
                self.dim.id(),
                [from.x, from.y, from.z],
                [to.x, to.y, to.z],
                &mut cancel,
            );
            !cancel
        };
        if self.player.is_some()
            && !self.lan_acting_fake
            && self.riding.is_some_and(&carried)
            && let Some(uuid) = self.lan_acting_uuid.or_else(|| {
                self.level
                    .player_uuid
                    .map(horizon_client::lan::playerdata::ints_uuid)
            })
            && !allows(uuid)
        {
            return false;
        }
        self.lan.as_ref().is_none_or(|host| {
            host.hub
                .guests
                .iter()
                .filter(|guest| {
                    Some(guest.conn) != self.lan_acting
                        && guest.dimension == self.dim.id()
                        && guest.placed
                        && !guest.dead
                        && !guest.is_fake()
                        && guest.vehicle.is_some_and(&carried)
                })
                .all(|guest| allows(guest.uuid))
        })
    }
}
