//! A portal transition belongs to the hull, even if none of its players can follow it.

use super::{Dim, Game, PLAYER_DELAY, Plan, PlayerTravelOrigin, TravelCause};
use crate::game::{
    minecarts::SEAT,
    vehicle_teleport::{absolute_pitch, passenger_destination},
};
use horizon_client::lan::guest::Guest;
use horizon_core::geometry::Vec3;

#[cfg(test)]
#[path = "hull_portal_tests.rs"]
mod tests;

#[derive(Clone, Copy)]
struct PortalRider {
    conn: u32,
    entity: u32,
    counter: i32,
    origin: PlayerTravelOrigin,
}

impl PortalRider {
    fn capture(guest: &Guest) -> Self {
        Self {
            conn: guest.conn,
            entity: guest.entity_id as u32,
            counter: guest.teleport_counter,
            origin: PlayerTravelOrigin::capture(guest),
        }
    }

    fn same_player(self, guest: &Guest) -> bool {
        guest.conn == self.conn
            && guest.entity_id as u32 == self.entity
            && guest.uuid == self.origin.uuid
            && guest.participates_in_player_simulation()
            && !guest.dead
            && guest.health > 0.0
    }

    fn still_seated(self, guest: &Guest, source: Dim) -> bool {
        self.same_player(guest)
            && guest.dimension == source.id()
            && guest.vehicle == self.origin.vehicle
    }

    fn still_at_origin(self, guest: &Guest, source: Dim) -> bool {
        self.still_seated(guest, source)
            && guest.teleport_counter == self.counter
            && self.origin.matches(guest, source)
    }
}

#[derive(Clone, Copy)]
struct PortalMob {
    entity: u32,
    uuid: [i32; 4],
}

#[derive(Clone, Copy)]
enum PortalPassenger {
    Player(PortalRider),
    Mob(PortalMob),
}

impl PortalPassenger {
    fn entity_id(self) -> u32 {
        match self {
            Self::Player(rider) => rider.entity,
            Self::Mob(rider) => rider.entity,
        }
    }
}

pub(in crate::game) struct PortalHullTrip {
    pub(super) source: Dim,
    hull: u32,
    source_chunk: (i32, i32),
    pub(super) plan: Plan,
    pub(super) target: Vec3,
    passengers: Vec<PortalPassenger>,
}

impl PortalHullTrip {
    pub(in crate::game) fn holds(&self, guest: &Guest) -> bool {
        self.passengers.iter().any(|passenger| {
            matches!(passenger, PortalPassenger::Player(rider) if rider.still_seated(guest, self.source))
        })
    }

    pub(super) fn pending_chunk(&self, dim: Dim) -> Option<(i32, i32)> {
        if dim == self.source {
            Some(self.source_chunk)
        } else {
            (self.plan.dim == dim).then(|| Game::chunk_of(self.target))
        }
    }
}

struct HullArrival {
    at: Vec3,
    yaw: f32,
    relative_yaw: bool,
    pitch_delta: f32,
}

impl HullArrival {
    fn rider_rotation(&self, hull_yaw: f32, [yaw, pitch]: [f32; 2]) -> [f32; 2] {
        let passenger_yaw = if self.relative_yaw {
            yaw
        } else {
            yaw - hull_yaw
        };
        [
            self.yaw + passenger_yaw,
            absolute_pitch(pitch + self.pitch_delta).unwrap_or(pitch),
        ]
    }

    fn hull_yaw(&self, source_yaw: f32) -> f32 {
        if self.relative_yaw {
            self.yaw + source_yaw
        } else {
            self.yaw
        }
    }
}

impl Game<'_> {
    /// The boat or cart owns its wait independently of the network players.
    /// Direct resident mobs can transfer alongside the directly seated players.
    pub(super) fn schedule_portal_hull(&mut self, hull: u32, plan: Plan, conns: &[u32]) -> bool {
        let source = self.dim;
        if self
            .portal_hull_trips
            .iter()
            .any(|trip| trip.source == source && trip.hull == hull)
        {
            return true;
        }
        let Some((at, _)) = self.portal_hull_pose(hull) else {
            return false;
        };
        let ids = self.native_passenger_ids(hull);
        let mob_count = self.mob_passenger_ids(hull).len();
        if ids.len() != conns.len() + mob_count
            || self.ordered_passenger_slots(hull).len() != mob_count
        {
            return false;
        }
        let passengers: Option<Vec<_>> = ids
            .into_iter()
            .map(|id| self.capture_portal_passenger(id, hull, conns))
            .collect();
        let Some(passengers) = passengers else {
            return false;
        };
        if !self.open_portal_hull_level(plan.dim) {
            return true;
        }
        let target = if plan.end {
            let p = plan.approximate;
            Vec3::new(f64::from(p.x) + 0.5, f64::from(p.y), f64::from(p.z) + 0.5)
        } else {
            self.arrival_centre(&plan)
        };
        if !self.open_portal_hull_level(source) {
            return true;
        }
        let trip = PortalHullTrip {
            source,
            hull,
            source_chunk: Self::chunk_of(at),
            plan,
            target,
            passengers,
        };
        if let Some(host) = self.lan.as_mut() {
            for i in 0..host.hub.guests.len() {
                if trip.holds(&host.hub.guests[i]) {
                    host.hub.guests.update(i, |guest| {
                        guest.landing = true;
                        guest.tp_held = true;
                    });
                }
            }
        }
        self.portal_hull_trips.push(trip);
        true
    }

    fn capture_portal_passenger(
        &mut self,
        id: u32,
        hull: u32,
        conns: &[u32],
    ) -> Option<PortalPassenger> {
        if let Some(guest) = self.lan.as_ref()?.hub.guests.iter().find(|guest| {
            guest.entity_id as u32 == id
                && conns.contains(&guest.conn)
                && guest.vehicle == Some(hull)
                && guest.dimension == self.dim.id()
                && guest.participates_in_player_simulation()
        }) {
            return self
                .native_passenger_ids(id)
                .is_empty()
                .then(|| PortalPassenger::Player(PortalRider::capture(guest)));
        }
        self.portal_mob_tree_is_resident(id).then(|| {
            PortalPassenger::Mob(PortalMob {
                entity: id,
                uuid: self.uuid_for(id),
            })
        })
    }

    /// `arrive_vehicle` moves mob objects and their attachments, including
    /// known descendants. An opaque saved slot or an indirect network player
    /// needs the original routing until that rider can be admitted separately.
    fn portal_mob_tree_is_resident(&self, root: u32) -> bool {
        std::iter::once(root)
            .chain(self.native_indirect_passenger_ids(root))
            .all(|id| {
                self.entity(id).is_some()
                    && self.mobs.iter().any(|mob| mob.id == id && mob.alive())
                    && self.ordered_passenger_slots(id).len() == self.mob_passenger_ids(id).len()
            })
    }

    fn portal_mob_is_current(&self, rider: PortalMob) -> bool {
        self.persist.uuids.get(&rider.entity) == Some(&rider.uuid)
            && self.portal_mob_tree_is_resident(rider.entity)
    }

    pub(super) fn arrive_portal_hulls(&mut self) {
        let mut i = 0;
        while let Some(trip) = self.portal_hull_trips.get(i) {
            if trip.plan.dim != self.dim {
                i += 1;
                continue;
            }
            let (cx, cz) = Self::chunk_of(trip.target);
            if !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                i += 1;
                continue;
            }
            let trip = self.portal_hull_trips.remove(i);
            self.arrive_portal_hull(trip);
        }
    }

    fn arrive_portal_hull(&mut self, trip: PortalHullTrip) {
        let destination = trip.plan.dim;
        let cause = TravelCause::of_plan(&trip.plan);
        self.resolve_arrivals();
        if self.dim != destination
            || self.parked_vehicle_root(trip.source, trip.hull) != Some(trip.hull)
        {
            return;
        }
        let Some(arrival) = self.resolve_portal_hull_exit(&trip.plan) else {
            return;
        };
        if self.dim != destination {
            return;
        }
        if !self.open_portal_hull_level(trip.source) {
            return;
        }
        let Some((hull_from, hull_yaw, passengers)) = self.detach_portal_hull_riders(&trip) else {
            self.open_portal_hull_level(destination);
            return;
        };

        // Paper ejects the original passenger list back to front, then teleports
        // each of those now-unmounted passengers in forward order. A subsequent
        // listener can observe an earlier accepted rider already in the destination.
        let mut landed = Vec::new();
        for passenger in passengers {
            if !self.open_portal_hull_level(trip.source) {
                break;
            }
            let rider = match passenger {
                PortalPassenger::Player(rider) => rider,
                PortalPassenger::Mob(rider) => {
                    if self.arrive_portal_mob(rider, &trip, hull_from, hull_yaw, &arrival) {
                        landed.push(passenger);
                    }
                    continue;
                }
            };
            let [x, y, z] = rider.origin.position;
            let destination_at = passenger_destination(hull_from, arrival.at, Vec3::new(x, y, z));
            let Some(rotation) =
                self.admit_portal_rider(rider, trip.source, destination, destination_at, cause)
            else {
                if self.portal_rider_is_current(rider, trip.source) {
                    self.cool_down_refused_portal(rider.conn);
                }
                continue;
            };
            if !self.open_portal_hull_level(destination) {
                break;
            }
            if let Some(rider) = self.land_portal_rider(
                rider,
                trip.source,
                destination_at,
                arrival.rider_rotation(hull_yaw, rotation),
            ) {
                landed.push(PortalPassenger::Player(rider));
            }
        }

        if !self.open_portal_hull_level(trip.source) {
            self.open_portal_hull_level(destination);
            return;
        }
        let hull_ready = self.portal_hull_pose(trip.hull) == Some((hull_from, hull_yaw))
            && self.native_passenger_ids(trip.hull).is_empty()
            && self.ordered_passenger_slots(trip.hull).is_empty();
        if hull_ready {
            self.release_vehicle_container(trip.hull);
        }
        if !self.open_portal_hull_level(destination) || !hull_ready {
            return;
        }
        self.arrive_vehicle(
            trip.source,
            trip.hull,
            arrival.at,
            arrival.hull_yaw(hull_yaw),
        );
        self.portals.carried_at.remove(&trip.hull);
        for passenger in landed {
            match passenger {
                PortalPassenger::Player(rider) => {
                    self.reattach_portal_rider(rider, destination, trip.hull);
                }
                PortalPassenger::Mob(rider) => {
                    if self.dim == destination
                        && self.portal_mob_is_current(rider)
                        && self.passenger_object_vehicle(rider.entity).is_none()
                        && let (Some(entity), Some(hull)) =
                            (self.entity(rider.entity), self.entity(trip.hull))
                    {
                        self.entity_start_riding(entity, hull, true);
                    }
                }
            }
        }
    }

    fn arrive_portal_mob(
        &mut self,
        rider: PortalMob,
        trip: &PortalHullTrip,
        hull_from: Vec3,
        hull_yaw: f32,
        arrival: &HullArrival,
    ) -> bool {
        let Some(mob) = self.mob_object(rider.entity) else {
            return false;
        };
        let from = mob.position;
        let destination = trip.plan.dim;
        if !self.portal_mob_is_current(rider)
            || self.passenger_object_vehicle(rider.entity).is_some()
            || !self.entity_may_travel(rider.entity, destination, TravelCause::of_plan(&trip.plan))
            || self.dim != trip.source
            || !self.portal_mob_is_current(rider)
        {
            return false;
        }
        let Some(mob) = self.mob_object(rider.entity) else {
            return false;
        };
        if mob.extra.mount.is_some() || mob.position != from {
            return false;
        }
        let to = passenger_destination(hull_from, arrival.at, from);
        let [yaw, pitch] = arrival.rider_rotation(hull_yaw, [mob.yaw, mob.head_pitch]);
        // PositionMoveRotation.calculateAbsolute rotates the relative delta by the
        // rider's own pitch and yaw changes, including when its reboarding is refused.
        let velocity = mob
            .velocity
            .rotate_x(f64::from(mob.head_pitch - pitch).to_radians() as f32)
            .rotate_y(f64::from(mob.yaw - yaw).to_radians() as f32);
        if !self.open_portal_hull_level(destination)
            || self.parked_vehicle_root(trip.source, rider.entity) != Some(rider.entity)
        {
            return false;
        }
        self.arrive_vehicle(trip.source, rider.entity, to, yaw);
        if let Some(mob) = self.mob_object_mut(rider.entity) {
            mob.velocity = velocity;
            mob.body_yaw = yaw;
            mob.head_yaw = yaw;
            mob.head_pitch = pitch;
            mob.previous_head_pitch = pitch;
        }
        self.portal_mob_is_current(rider)
    }

    fn resolve_portal_hull_exit(&mut self, plan: &Plan) -> Option<HullArrival> {
        if plan.end {
            // The transitioning entity is the hull, so the entrance uses the
            // full End anchor and the return uses the shared world spawn.
            let at = self.end_portal_exit(plan);
            if self.dim != plan.dim {
                return None;
            }
            self.place_portal_ticket(Self::chunk_of(at));
            let into_end = plan.dim == Dim::End;
            let [yaw, pitch_delta] = if into_end {
                [90.0, 0.0]
            } else {
                self.level.world_spawn_angle
            };
            return Some(HullArrival {
                at,
                yaw,
                relative_yaw: !into_end,
                pitch_delta,
            });
        }
        let exit = self.nether_exit_portal(plan)?;
        let (at, yaw) = self.arrival(plan, exit);
        Some(HullArrival {
            at,
            yaw,
            relative_yaw: true,
            pitch_delta: 0.0,
        })
    }

    fn reattach_portal_rider(&mut self, rider: PortalRider, destination: Dim, hull: u32) {
        if !self.portal_rider_is_current(rider, destination) {
            return;
        }
        let (Some(entity), Some(vehicle)) = (self.entity(rider.entity), self.entity(hull)) else {
            return;
        };
        if !self.entity_start_riding(entity, vehicle, true) || self.dim != destination {
            return;
        }
        let (at, seat_at) = if let Some(i) = self.boats.iter().position(|boat| boat.id == hull) {
            let Some(seat_at) = self.guest_boat_seat(i, rider.entity) else {
                return;
            };
            (self.boats[i].position, seat_at)
        } else if let Some(cart) = self.carts.iter().find(|cart| cart.id == hull) {
            (
                cart.position,
                cart.position.add_vector(Vec3::new(0.0, SEAT, 0.0)),
            )
        } else {
            return;
        };
        let Some(i) = self.guest_index(rider.conn) else {
            return;
        };
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let guest = &host.hub.guests[i];
        if !rider.same_player(guest)
            || guest.dimension != destination.id()
            || guest.vehicle != Some(hull)
        {
            return;
        }
        // ServerPlayer.startRiding positions the rider before setting the
        // connection's teleport baseline. The hull tracker publishes its seats.
        host.hub.guests.update(i, |guest| {
            guest.vehicle_good = Some(at.components());
            guest.on_ground = false;
        });
        host.hub.teleport(i, Some(seat_at.components()));
    }

    fn detach_portal_hull_riders(
        &mut self,
        trip: &PortalHullTrip,
    ) -> Option<(Vec3, f32, Vec<PortalPassenger>)> {
        let root = self.entity(trip.hull)?;
        let hull_pose = self.portal_hull_pose(trip.hull)?;
        if self.entity_travel_hook_epoch().is_some() {
            let mut cancel = false;
            self.native_entity_travel_to_dimension(
                root,
                trip.plan.dim.id(),
                TravelCause::of_plan(&trip.plan).id(),
                &mut cancel,
            );
            if cancel
                || self.dim != trip.source
                || self.portal_hull_pose(trip.hull) != Some(hull_pose)
            {
                return None;
            }
        }
        let passengers: Vec<_> = trip
            .passengers
            .iter()
            .copied()
            .filter(|passenger| match *passenger {
                PortalPassenger::Player(rider) => {
                    !self.on_server_trip(rider.conn)
                        && self
                            .lan
                            .as_ref()
                            .and_then(|host| host.hub.guest(rider.conn))
                            .is_some_and(|guest| rider.still_seated(guest, trip.source))
                }
                PortalPassenger::Mob(rider) => {
                    self.portal_mob_is_current(rider)
                        && self.passenger_object_vehicle(rider.entity) == Some(trip.hull)
                }
            })
            .collect();
        if self
            .native_passenger_ids(trip.hull)
            .into_iter()
            .ne(passengers.iter().copied().map(PortalPassenger::entity_id))
            || self.ordered_passenger_slots(trip.hull).len()
                != passengers
                    .iter()
                    .filter(|passenger| matches!(passenger, PortalPassenger::Mob(_)))
                    .count()
        {
            return None;
        }
        for passenger in passengers.iter().rev() {
            if self.dim != trip.source || self.portal_hull_pose(trip.hull) != Some(hull_pose) {
                return None;
            }
            if let Some(entity) = self.entity(passenger.entity_id())
                && self.entity_vehicle(entity) == Some(root)
            {
                self.entity_stop_riding(entity);
            }
        }
        if self.dim != trip.source
            || self.portal_hull_pose(trip.hull) != Some(hull_pose)
            || !self.native_passenger_ids(trip.hull).is_empty()
            || !self.ordered_passenger_slots(trip.hull).is_empty()
        {
            return None;
        }
        let host = self.lan.as_ref()?;
        let detached = passengers
            .into_iter()
            .filter_map(|passenger| match passenger {
                PortalPassenger::Player(rider) => {
                    let guest = host.hub.guest(rider.conn)?;
                    (rider.same_player(guest)
                        && guest.dimension == trip.source.id()
                        && guest.vehicle.is_none())
                    .then(|| PortalPassenger::Player(PortalRider::capture(guest)))
                }
                PortalPassenger::Mob(rider) => (self.portal_mob_is_current(rider)
                    && self.passenger_object_vehicle(rider.entity).is_none())
                .then_some(passenger),
            })
            .collect();
        Some((hull_pose.0, hull_pose.1, detached))
    }

    fn portal_rider_is_current(&self, rider: PortalRider, source: Dim) -> bool {
        !self.on_server_trip(rider.conn)
            && self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(rider.conn))
                .is_some_and(|guest| rider.still_at_origin(guest, source))
    }

    fn admit_portal_rider(
        &mut self,
        rider: PortalRider,
        source: Dim,
        destination: Dim,
        at: Vec3,
        cause: TravelCause,
    ) -> Option<[f32; 2]> {
        if !self.portal_rider_is_current(rider, source)
            || !self.server_player_travel_allowed(rider.conn, destination, cause)
            || self.dim != source
            || !self.portal_rider_is_current(rider, source)
            || !self.player_teleport_allowed(
                rider.origin.uuid,
                (source, rider.origin.position),
                (destination, at.components()),
                cause.id(),
            )
            || self.dim != source
        {
            return None;
        }
        let guest = self.lan.as_ref()?.hub.guest(rider.conn)?;
        (!self.on_server_trip(rider.conn) && rider.still_at_origin(guest, source))
            .then_some([guest.yaw, guest.pitch])
    }

    fn land_portal_rider(
        &mut self,
        rider: PortalRider,
        source: Dim,
        at: Vec3,
        [yaw, pitch]: [f32; 2],
    ) -> Option<PortalRider> {
        if !self.portal_rider_is_current(rider, source) {
            return None;
        }
        let destination = self.dim;
        let [x, y, z] = rider.origin.position;
        self.place_portal_guest(
            rider.conn,
            source,
            Vec3::new(x, y, z),
            at,
            yaw,
            Some(pitch),
            false,
        );
        let i = self.guest_index(rider.conn)?;
        let host = self.lan.as_mut()?;
        let guest = &host.hub.guests[i];
        if !rider.same_player(guest)
            || guest.dimension != destination.id()
            || guest.vehicle.is_some()
            || guest.position != at.components()
        {
            return None;
        }
        host.play.entry(rider.conn).or_default().portal.cooldown = PLAYER_DELAY;
        host.hub.guests.update(i, |guest| {
            guest.new_body(true);
        });
        host.hub.land(i, at.components());
        let arrived = PortalRider::capture(&host.hub.guests[i]);
        self.notify_portal_arrival(rider.conn, source, Some(rider.origin.position), at);
        self.tell_mods_changed_dimension(rider.conn, Some(source), destination);
        Some(arrived)
    }

    fn open_portal_hull_level(&mut self, dim: Dim) -> bool {
        if let Err(error) = self.enter_level(dim) {
            log::error!("could not open {} for a portal hull: {error}", dim.id());
            return false;
        }
        self.dim == dim
    }
}
