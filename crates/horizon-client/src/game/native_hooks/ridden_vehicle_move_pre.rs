use super::*;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(in crate::game) fn native_ridden_vehicle_move_pre(
        &self,
        call: mod_api::__private::RiddenVehicleMovePreCall<'_>,
    ) {
        self.fire_ridden_vehicle_move_pre(call);
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

    pub(in crate::game) fn ridden_vehicle_packet_allowed(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
        guest_driver: u128,
        driver_move_from: Option<[f64; 3]>,
    ) -> bool {
        self.ridden_move_allowed_with(
            vehicle,
            from,
            to,
            Some(guest_driver),
            driver_move_from,
            |seat| self.vehicle_carries_seat(vehicle, seat),
        )
    }

    pub(in crate::game) fn ridden_vehicle_move_allowed_with(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
        carried: impl Fn(u32) -> bool,
    ) -> bool {
        self.ridden_move_allowed_with(vehicle, from, to, None, None, carried)
    }

    fn ridden_move_allowed_with(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
        guest_driver: Option<u128>,
        driver_move_from: Option<[f64; 3]>,
        carried: impl Fn(u32) -> bool,
    ) -> bool {
        if !self.checks_ridden_move(from != to || driver_move_from.is_some()) {
            return true;
        }
        let allows = |uuid| {
            self.ridden_move_denial(vehicle, from, to, uuid, guest_driver, driver_move_from)
                .is_none()
        };
        if self.player.is_some()
            && !self.lan_acting_fake
            && self.riding.is_some_and(&carried)
            && let Some(uuid) = self.riding_player_uuid()
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

    /// The first denied rider in passenger-tree order and their rollback request.
    pub(in crate::game) fn ridden_hull_move_denial(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
    ) -> Option<(u128, bool)> {
        if !self.checks_ridden_move(from != to) {
            return None;
        }
        self.hull_rider_denial_with(vehicle, |uuid| {
            self.ridden_move_denial(vehicle, from, to, uuid, None, None)
        })
    }

    pub(in crate::game) fn guest_boat_tick_move_denial(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
    ) -> Option<u128> {
        if from == to
            || !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::GuestBoatTickMovePre)
        {
            return None;
        }
        self.hull_rider_denial_with(vehicle, |player| {
            let mut cancel = false;
            self.native_guest_boat_tick_move_pre(mod_api::__private::GuestBoatTickMovePreCall {
                player,
                vehicle,
                dimension: self.dim.id(),
                from: from.components(),
                to: to.components(),
                cancel: &mut cancel,
            });
            cancel.then_some(())
        })
        .map(|(player, ())| player)
    }

    fn hull_rider_denial_with<T>(
        &self,
        vehicle: u32,
        deny: impl Fn(u128) -> Option<T>,
    ) -> Option<(u128, T)> {
        let mut order = self.native_indirect_passenger_ids(vehicle);
        let acting = self.player.as_ref().and_then(|body| {
            let seat = self.riding?;
            if self.lan_acting_fake || !self.vehicle_carries_seat(vehicle, seat) {
                return None;
            }
            Some((
                u32::try_from(body.entity_id).ok()?,
                self.riding_player_uuid()?,
                seat,
            ))
        });
        if let Some((id, _, seat)) = acting
            && !order.contains(&id)
        {
            // An integrated host player without a guest seat record still rides.
            let index = order
                .iter()
                .position(|&rider| rider == seat)
                .map_or(0, |i| i + 1);
            order.insert(index, id);
        }
        order.into_iter().find_map(|id| {
            let uuid = if let Some((active, uuid, _)) = acting
                && id == active
            {
                uuid
            } else {
                let guest = self.lan.as_ref()?.hub.guests.iter().find(|guest| {
                    Some(guest.conn) != self.lan_acting
                        && guest.entity_id as u32 == id
                        && guest.dimension == self.dim.id()
                        && guest.placed
                        && !guest.dead
                        && !guest.is_fake()
                })?;
                guest.uuid
            };
            deny(uuid).map(|denial| (uuid, denial))
        })
    }

    fn checks_ridden_move(&self, moved: bool) -> bool {
        moved && self.native_authoritative() && self.ridden_move_listened()
    }

    pub(in crate::game) fn ridden_move_listened(&self) -> bool {
        mod_api::__private::listened(mod_api::HookId::RiddenVehicleMovePre)
    }

    fn riding_player_uuid(&self) -> Option<u128> {
        self.lan_acting_uuid.or_else(|| {
            self.level
                .player_uuid
                .map(horizon_client::lan::playerdata::ints_uuid)
        })
    }

    fn ridden_move_denial(
        &self,
        vehicle: u32,
        from: Vec3,
        to: Vec3,
        player: u128,
        guest_driver: Option<u128>,
        driver_move_from: Option<[f64; 3]>,
    ) -> Option<bool> {
        let mut cancel = false;
        let mut paper_rollback = false;
        self.native_ridden_vehicle_move_pre(mod_api::__private::RiddenVehicleMovePreCall {
            player,
            vehicle,
            dimension: self.dim.id(),
            from: from.components(),
            to: to.components(),
            guest_driver,
            driver_move_from,
            paper_rollback: &mut paper_rollback,
            cancel: &mut cancel,
        });
        cancel.then_some(paper_rollback)
    }
}
