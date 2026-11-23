//! A Nether transition belongs to the hull, even if none of its players can follow it.

use super::{Dim, Game, PLAYER_DELAY, Plan, PlayerTravelOrigin, TravelCause};
use crate::game::{minecarts::SEAT, vehicle_teleport::passenger_destination};
use horizon_client::lan::guest::Guest;
use horizon_core::geometry::Vec3;

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

pub(in crate::game) struct NetherHullTrip {
    pub(super) source: Dim,
    hull: u32,
    source_chunk: (i32, i32),
    pub(super) plan: Plan,
    pub(super) target: Vec3,
    riders: Vec<PortalRider>,
}

impl NetherHullTrip {
    pub(in crate::game) fn holds(&self, guest: &Guest) -> bool {
        self.riders
            .iter()
            .any(|rider| rider.still_seated(guest, self.source))
    }

    pub(super) fn pending_chunk(&self, dim: Dim) -> Option<(i32, i32)> {
        if dim == self.source {
            Some(self.source_chunk)
        } else {
            (self.plan.dim == dim).then(|| Game::chunk_of(self.target))
        }
    }
}

impl Game<'_> {
    /// The existing entity tree transfer handles mobs and nested mounts. A
    /// boat or cart whose seats contain only network players owns its wait
    /// independently of any of those connections.
    pub(super) fn schedule_nether_hull(&mut self, hull: u32, plan: Plan, conns: &[u32]) -> bool {
        let source = self.dim;
        if self
            .nether_hull_trips
            .iter()
            .any(|trip| trip.source == source && trip.hull == hull)
        {
            return true;
        }
        let Some((at, _)) = self.portal_hull_pose(hull) else {
            return false;
        };
        if !self.ordered_passenger_slots(hull).is_empty() {
            return false;
        }
        let ids = self.native_passenger_ids(hull);
        if ids.len() != conns.len() {
            return false;
        }
        let riders: Option<Vec<_>> = ids
            .into_iter()
            .map(|id| {
                let guests = &self.lan.as_ref()?.hub.guests;
                guests
                    .iter()
                    .find(|guest| {
                        guest.entity_id as u32 == id
                            && conns.contains(&guest.conn)
                            && guest.vehicle == Some(hull)
                            && guest.dimension == source.id()
                            && guest.participates_in_player_simulation()
                            && self.native_passenger_ids(id).is_empty()
                    })
                    .map(PortalRider::capture)
            })
            .collect();
        let Some(riders) = riders else { return false };
        if !self.open_nether_hull_level(plan.dim) {
            return true;
        }
        let target = self.arrival_centre(&plan);
        if !self.open_nether_hull_level(source) {
            return true;
        }
        let trip = NetherHullTrip {
            source,
            hull,
            source_chunk: Self::chunk_of(at),
            plan,
            target,
            riders,
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
        self.nether_hull_trips.push(trip);
        true
    }

    pub(super) fn arrive_nether_hulls(&mut self) {
        let mut i = 0;
        while let Some(trip) = self.nether_hull_trips.get(i) {
            if trip.plan.dim != self.dim {
                i += 1;
                continue;
            }
            let (cx, cz) = Self::chunk_of(trip.target);
            if !(-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                i += 1;
                continue;
            }
            let trip = self.nether_hull_trips.remove(i);
            self.arrive_nether_hull(trip);
        }
    }

    fn arrive_nether_hull(&mut self, trip: NetherHullTrip) {
        let destination = trip.plan.dim;
        self.resolve_arrivals();
        let Some(exit) = self.nether_exit_portal(&trip.plan) else {
            return;
        };
        if self.dim != destination
            || self.parked_vehicle_root(trip.source, trip.hull) != Some(trip.hull)
        {
            return;
        }
        let (at, turn) = self.arrival(&trip.plan, exit);
        if !self.open_nether_hull_level(trip.source) {
            return;
        }
        let Some((hull_from, hull_yaw, riders)) = self.detach_nether_hull_riders(&trip) else {
            self.open_nether_hull_level(destination);
            return;
        };

        // Paper ejects the original passenger list back to front, then teleports
        // each of those now-unmounted passengers in forward order. A subsequent
        // listener can observe an earlier accepted rider already in the destination.
        let mut landed = Vec::new();
        for rider in riders {
            if !self.open_nether_hull_level(trip.source) {
                break;
            }
            let [x, y, z] = rider.origin.position;
            let destination_at = passenger_destination(hull_from, at, Vec3::new(x, y, z));
            let Some(yaw) =
                self.admit_nether_rider(rider, trip.source, destination, destination_at)
            else {
                if self.nether_rider_is_current(rider, trip.source) {
                    self.cool_down_refused_portal(rider.conn);
                }
                continue;
            };
            if !self.open_nether_hull_level(destination) {
                break;
            }
            if let Some(rider) =
                self.land_nether_rider(rider, trip.source, destination_at, yaw + turn)
            {
                landed.push(rider);
            }
        }

        if !self.open_nether_hull_level(trip.source) {
            self.open_nether_hull_level(destination);
            return;
        }
        let hull_ready = self.portal_hull_pose(trip.hull) == Some((hull_from, hull_yaw))
            && self.native_passenger_ids(trip.hull).is_empty()
            && self.ordered_passenger_slots(trip.hull).is_empty();
        if hull_ready {
            self.release_vehicle_container(trip.hull);
        }
        if !self.open_nether_hull_level(destination) || !hull_ready {
            return;
        }
        self.arrive_vehicle(trip.source, trip.hull, at, hull_yaw + turn);
        self.portals.carried_at.remove(&trip.hull);
        for rider in landed {
            self.reattach_nether_rider(rider, destination, trip.hull);
        }
    }

    fn reattach_nether_rider(&mut self, rider: PortalRider, destination: Dim, hull: u32) {
        if !self.nether_rider_is_current(rider, destination) {
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

    fn detach_nether_hull_riders(
        &mut self,
        trip: &NetherHullTrip,
    ) -> Option<(Vec3, f32, Vec<PortalRider>)> {
        let root = self.entity(trip.hull)?;
        let hull_pose = self.portal_hull_pose(trip.hull)?;
        if self.entity_travel_hook_epoch().is_some() {
            let mut cancel = false;
            self.native_entity_travel_to_dimension(
                root,
                trip.plan.dim.id(),
                TravelCause::NetherPortal.id(),
                &mut cancel,
            );
            if cancel
                || self.dim != trip.source
                || self.portal_hull_pose(trip.hull) != Some(hull_pose)
            {
                return None;
            }
        }
        let riders: Vec<_> = trip
            .riders
            .iter()
            .copied()
            .filter(|rider| {
                !self.on_server_trip(rider.conn)
                    && self
                        .lan
                        .as_ref()
                        .and_then(|host| host.hub.guest(rider.conn))
                        .is_some_and(|guest| rider.still_seated(guest, trip.source))
            })
            .collect();
        if self
            .native_passenger_ids(trip.hull)
            .into_iter()
            .ne(riders.iter().map(|rider| rider.entity))
            || !self.ordered_passenger_slots(trip.hull).is_empty()
        {
            return None;
        }
        for rider in riders.iter().rev() {
            if self.dim != trip.source || self.portal_hull_pose(trip.hull) != Some(hull_pose) {
                return None;
            }
            if let Some(entity) = self.entity(rider.entity)
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
        let detached = riders
            .into_iter()
            .filter_map(|rider| {
                let guest = host.hub.guest(rider.conn)?;
                (rider.same_player(guest)
                    && guest.dimension == trip.source.id()
                    && guest.vehicle.is_none())
                .then(|| PortalRider::capture(guest))
            })
            .collect();
        Some((hull_pose.0, hull_pose.1, detached))
    }

    fn nether_rider_is_current(&self, rider: PortalRider, source: Dim) -> bool {
        !self.on_server_trip(rider.conn)
            && self
                .lan
                .as_ref()
                .and_then(|host| host.hub.guest(rider.conn))
                .is_some_and(|guest| rider.still_at_origin(guest, source))
    }

    fn admit_nether_rider(
        &mut self,
        rider: PortalRider,
        source: Dim,
        destination: Dim,
        at: Vec3,
    ) -> Option<f32> {
        let cause = TravelCause::NetherPortal;
        if !self.nether_rider_is_current(rider, source)
            || !self.server_player_travel_allowed(rider.conn, destination, cause)
            || self.dim != source
            || !self.nether_rider_is_current(rider, source)
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
            .then_some(guest.yaw)
    }

    fn land_nether_rider(
        &mut self,
        rider: PortalRider,
        source: Dim,
        at: Vec3,
        yaw: f32,
    ) -> Option<PortalRider> {
        if !self.nether_rider_is_current(rider, source) {
            return None;
        }
        let destination = self.dim;
        let [x, y, z] = rider.origin.position;
        self.place_portal_guest(rider.conn, source, Vec3::new(x, y, z), at, yaw, false);
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

    fn open_nether_hull_level(&mut self, dim: Dim) -> bool {
        if let Err(error) = self.enter_level(dim) {
            log::error!(
                "could not open {} for a Nether portal hull: {error}",
                dim.id()
            );
            return false;
        }
        self.dim == dim
    }
}
