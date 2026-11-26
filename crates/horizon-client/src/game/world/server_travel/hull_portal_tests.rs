use super::*;
use crate::game::{
    boats::Boat,
    native_entity_refs::tests::{prepare_entity_ticking_neighborhood, run_on_guest_stack},
    native_guest_turn_tests::with_guests,
};
use horizon_client::{chunk::Chunk, mobs::Mob, nether_portal::ENTITY_DELAY};
use horizon_core::position::{BlockPos, ChunkPos};
use mod_api::{
    EntityMountEvent, EntityTravelToDimensionEvent, Flow, Hooks, ModEntry,
    PlayerPositionCommittedEvent, PlayerTeleportPreEvent, ServerHost,
};
use std::sync::{Arc, Mutex};

const HULL: u32 = 702;
const PIG: u32 = 703;
const GUEST: u32 = 12;
const CHILD: u32 = 704;
const ENTRY: Vec3 = Vec3::new(3.5, 70.0, 5.5);
const END_SPAWN: Vec3 = Vec3::new(100.5, 50.0, 0.5);
const END_RETURN: Vec3 = Vec3::new(6.5, 69.0, 9.5);

#[derive(Clone, Copy)]
enum Route {
    IntoEnd,
    OutOfEnd,
    Nether,
    UnseenCredits,
}

impl Route {
    fn levels(self) -> (Dim, Dim, TravelCause) {
        match self {
            Self::IntoEnd => (Dim::Overworld, Dim::End, TravelCause::EndPortal),
            Self::OutOfEnd | Self::UnseenCredits => {
                (Dim::End, Dim::Overworld, TravelCause::EndPortal)
            }
            Self::Nether => (Dim::Overworld, Dim::Nether, TravelCause::NetherPortal),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum Step {
    Travel(u32),
    Dismount(u32),
    Mount(u32),
    OfferGuest,
    CommitGuest,
}

struct DestinationPolicy {
    refuse_guest: bool,
    refuse_mob: bool,
    nested_guest: bool,
    source: Dim,
    destination: Dim,
    cause: TravelCause,
    hull_arrival: Arc<Mutex<Option<Vec3>>>,
    steps: Arc<Mutex<Vec<Step>>>,
}

#[mod_api::__subscriptions]
impl Hooks for DestinationPolicy {
    fn entity_travel_to_dimension(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityTravelToDimensionEvent,
    ) -> Flow {
        assert_eq!(event.dimension, self.destination.id());
        assert_eq!(event.cause, self.cause.id());
        let id = event.entity.id();
        if id != HULL {
            assert_eq!(event.entity.vehicle(host), None);
        }
        self.steps.lock().unwrap().push(Step::Travel(id));
        if id == PIG && self.refuse_mob {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
        self.steps.lock().unwrap().push(if event.mounting {
            Step::Mount(event.entity)
        } else {
            Step::Dismount(event.entity)
        });
        Flow::Continue
    }

    fn player_teleport_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerTeleportPreEvent<'_>,
    ) -> Flow {
        if event.cause != self.cause.id() {
            return Flow::Continue;
        }
        assert_eq!(event.player, u128::from(GUEST));
        assert_eq!(event.from_level, self.source.id());
        assert_eq!(event.to_level, self.destination.id());
        let rider = host.player(event.player).unwrap().entity();
        assert_eq!(rider.position(host), Some(event.from));
        assert_eq!(rider.vehicle(host), None);
        let [x, y, z] = event.from;
        let expected = passenger_destination(
            ENTRY,
            self.hull_arrival.lock().unwrap().unwrap(),
            Vec3::new(x, y, z),
        )
        .components();
        if self.nested_guest {
            // Each link in the passenger tree adds its own world-space offset.
            assert!(
                event
                    .to
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| (a - b).abs() < 1e-9)
            );
        } else {
            assert_eq!(event.to, expected);
        }
        self.steps.lock().unwrap().push(Step::OfferGuest);
        if self.refuse_guest {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn player_position_committed(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerPositionCommittedEvent<'_>,
    ) {
        if event.from_level != event.to_level {
            assert_eq!(event.player, u128::from(GUEST));
            assert_eq!(event.to_level, self.destination.id());
            assert_eq!(
                host.player(event.player).unwrap().entity().vehicle(host),
                None
            );
            self.steps.lock().unwrap().push(Step::CommitGuest);
        }
    }
}

#[test]
#[ignore = "daily"]
fn natural_portal_boat_admits_the_guest_and_mob_separately_in_passenger_order() {
    run_on_guest_stack(|| {
        use Route::*;
        for (route, refuse_guest, refuse_mob, nested_mob, nested_guest) in [
            (IntoEnd, true, false, false, false),
            (IntoEnd, false, true, false, false),
            (IntoEnd, false, false, false, false),
            (IntoEnd, true, true, true, false),
            (Nether, true, false, false, false),
            (Nether, false, false, false, false),
            (OutOfEnd, true, false, false, false),
            (OutOfEnd, false, false, false, false),
            (UnseenCredits, false, false, false, false),
            (IntoEnd, true, false, false, true),
            (IntoEnd, false, true, false, true),
            (Nether, false, false, false, true),
            (UnseenCredits, false, false, false, true),
        ] {
            let (source, destination, cause) = route.levels();
            let hull_arrival = Arc::new(Mutex::new(None));
            let steps = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "mixed_portal_policy",
                Box::new(DestinationPolicy {
                    refuse_guest,
                    refuse_mob,
                    nested_guest,
                    source,
                    destination,
                    cause,
                    hull_arrival: hull_arrival.clone(),
                    steps: steps.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.enter_level(source).unwrap();
                    game.level.world_spawn = Some([6, 80, 9]);
                    game.level.world_spawn_angle = [25.0, -12.0];
                    let entry = BlockPos::new(3, 70, 5);
                    prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                    game.tickets.add_portal((0, 0));
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(entry.x, entry.z));
                    let registry = &game.pack.assets.registry;
                    let portal = registry
                        .default_state(if matches!(route, Nether) {
                            "minecraft:nether_portal"
                        } else {
                            "minecraft:end_portal"
                        })
                        .unwrap();
                    let nether_exit = matches!(route, Nether).then(|| {
                        let mut state = registry.state(portal).unwrap().clone();
                        state.properties.insert("axis".into(), "z".into());
                        registry.resolve(&state).unwrap()
                    });
                    let floor = registry.default_state("minecraft:stone").unwrap();
                    for x in 1..=5 {
                        for z in 3..=7 {
                            game.world.set(x, 69, z, floor).unwrap();
                        }
                    }
                    game.world.set(entry.x, entry.y, entry.z, portal).unwrap();
                    game.boats
                        .push(Boat::new(HULL, Arc::from("oak"), ENTRY, 35.0));
                    let species = game.res.mob_types()["minecraft:pig"].clone();
                    let mut body = Mob::new(PIG, species, ENTRY, 10.0);
                    if nested_guest {
                        body.set_saddle(Some(horizon_client::items::ItemStack::new(
                            "minecraft:saddle",
                            1,
                        )));
                    }
                    game.mobs.push(body);
                    let pig = game.entity(PIG).unwrap();
                    let hull = game.entity(HULL).unwrap();
                    let pig_uuid = game.entity_uuid(pig).unwrap();
                    assert!(game.entity_start_riding(pig, hull, false));
                    if nested_mob {
                        let species = game.res.mob_types()["minecraft:chicken"].clone();
                        game.mobs.push(Mob::new(CHILD, species, ENTRY, 0.0));
                        let child = game.entity(CHILD).unwrap();
                        assert!(game.entity_start_riding(child, pig, true));
                    }
                    game.lan.as_mut().unwrap().hub.update_guest(GUEST, |guest| {
                        guest.dimension = source.id().into();
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.seen_credits = !matches!(route, UnseenCredits);
                        if source == Dim::End {
                            guest.respawn_elsewhere =
                                Some((Dim::Overworld.id().into(), [54, 72, 47], 70.0, 10.0, true));
                        }
                    });
                    let rider = game.player(u128::from(GUEST)).unwrap().entity();
                    let mount = if nested_guest { pig } else { hull };
                    assert!(game.entity_start_riding(rider, mount, !nested_guest));
                    game.guest_boarded(GUEST, None);
                    let source_passengers: &[u32] =
                        if nested_guest { &[PIG] } else { &[GUEST, PIG] };
                    assert_eq!(game.native_passenger_ids(HULL), source_passengers);
                    if nested_guest {
                        assert_eq!(game.native_passenger_ids(PIG), [GUEST]);
                    }
                    steps.lock().unwrap().clear();

                    // Run the actual portal processor and carrier scheduler.
                    game.portals
                        .entities
                        .entry(HULL)
                        .or_default()
                        .set_inside(entry, ENTITY_DELAY);
                    game.handle_vehicle_portals();
                    game.carry_riders_through_portals();
                    assert!(game.server_trips.is_empty());
                    if matches!(route, UnseenCredits) && !nested_guest {
                        assert!(game.portal_hull_trips.is_empty());
                        assert_eq!(game.native_passenger_ids(HULL), source_passengers);
                        assert!(steps.lock().unwrap().is_empty());
                        return;
                    }
                    assert_eq!(game.portal_hull_trips.len(), 1);
                    assert!(game.on_server_trip(GUEST));
                    assert!(game.vehicle_waits_in(source));
                    assert_eq!(game.native_passenger_ids(HULL), source_passengers);
                    assert!(steps.lock().unwrap().is_empty());

                    let trip = &game.portal_hull_trips[0];
                    let plan = trip.plan;
                    let (cx, cz) = Game::chunk_of(trip.target);
                    game.enter_level(destination).unwrap();
                    game.arrive_server_trips();
                    assert_eq!(game.portal_hull_trips.len(), 1);
                    assert!(game.entity(HULL).is_none());
                    assert!(game.entity(PIG).is_none());
                    assert!(steps.lock().unwrap().is_empty());
                    game.enter_level(source).unwrap();
                    assert_eq!(game.native_passenger_ids(HULL), source_passengers);

                    game.enter_level(destination).unwrap();
                    for z in cz - 2..=cz + 2 {
                        for x in cx - 2..=cx + 2 {
                            game.streaming
                                .insert_remote(&mut game.world, Chunk::empty(x, z));
                        }
                    }
                    let (expected_hull, expected_yaw) = match route {
                        IntoEnd => (END_SPAWN, 90.0),
                        OutOfEnd | UnseenCredits => {
                            game.world.set(6, 68, 9, floor).unwrap();
                            (END_RETURN, 60.0)
                        }
                        Nether => {
                            let (exit_x, exit_z) = (plan.approximate.x + 2, plan.approximate.z + 2);
                            for y in 70..73 {
                                for z in exit_z..exit_z + 2 {
                                    game.world.set(exit_x, y, z, nether_exit.unwrap()).unwrap();
                                }
                            }
                            let (_, exit) =
                                game.find_exit_portal(&plan).expect("pre-existing portal");
                            let (at, turn) = game.arrival(&plan, exit);
                            assert_eq!(turn, 90.0);
                            (at, 35.0 + turn)
                        }
                    };
                    *hull_arrival.lock().unwrap() = Some(expected_hull);
                    game.arrive_server_trips();
                    assert!(game.portal_hull_trips.is_empty());
                    assert_eq!(
                        game.portal_hull_pose(HULL),
                        Some((expected_hull, expected_yaw))
                    );
                    assert_eq!(game.boats.iter().filter(|boat| boat.id == HULL).count(), 1);
                    let mut expected = vec![Step::Travel(HULL), Step::Dismount(PIG)];
                    let mut passengers = Vec::new();
                    if nested_guest {
                        expected.push(Step::Travel(PIG));
                        if !refuse_mob {
                            expected.extend([
                                Step::Dismount(GUEST),
                                Step::Travel(GUEST),
                                Step::OfferGuest,
                            ]);
                            if !refuse_guest {
                                expected.extend([Step::CommitGuest, Step::Mount(GUEST)]);
                            }
                            expected.push(Step::Mount(PIG));
                            passengers.push(PIG);
                        }
                    } else {
                        expected.extend([
                            Step::Dismount(GUEST),
                            Step::Travel(GUEST),
                            Step::OfferGuest,
                        ]);
                        if !refuse_guest {
                            expected.push(Step::CommitGuest);
                        }
                        expected.push(Step::Travel(PIG));
                        for (id, refused) in [(GUEST, refuse_guest), (PIG, refuse_mob)] {
                            if !refused {
                                expected.push(Step::Mount(id));
                                passengers.push(id);
                            }
                        }
                    }
                    assert_eq!(*steps.lock().unwrap(), expected);
                    assert_eq!(game.native_passenger_ids(HULL), passengers);
                    if refuse_mob {
                        assert!(game.mob_object(PIG).is_none());
                    } else {
                        let pig = game.entity(PIG).unwrap();
                        assert_eq!(game.entity_uuid(pig), Some(pig_uuid));
                        assert_eq!(game.mob_object(PIG).unwrap().extra.mount, Some(HULL));
                    }
                    let guest = game.lan.as_ref().unwrap().hub.guest(GUEST).unwrap();
                    assert!(!guest.landing && !guest.tp_held);
                    if nested_guest && refuse_mob {
                        assert_eq!(guest.dimension, source.id());
                        assert_eq!(guest.vehicle, Some(PIG));
                    } else if refuse_guest {
                        assert_eq!(guest.dimension, source.id());
                        assert_eq!(guest.vehicle, None);
                    } else {
                        assert_eq!(guest.dimension, destination.id());
                        assert_eq!(guest.vehicle, Some(if nested_guest { PIG } else { HULL }));
                    }

                    game.enter_level(source).unwrap();
                    assert!(game.portal_hull_pose(HULL).is_none());
                    if refuse_mob {
                        let pig = game.entity(PIG).unwrap();
                        assert_eq!(game.entity_uuid(pig), Some(pig_uuid));
                        assert_eq!(game.mob_object(PIG).unwrap().extra.mount, None);
                        if nested_mob {
                            assert_eq!(game.native_passenger_ids(PIG), [CHILD]);
                            assert_eq!(game.mob_object(CHILD).unwrap().extra.mount, Some(PIG));
                        }
                    } else {
                        assert!(game.mob_object(PIG).is_none());
                    }
                });
            });
        }
    });
}
