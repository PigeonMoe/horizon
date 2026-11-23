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
    steps: Arc<Mutex<Vec<Step>>>,
}

#[mod_api::__subscriptions]
impl Hooks for DestinationPolicy {
    fn entity_travel_to_dimension(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityTravelToDimensionEvent,
    ) -> Flow {
        assert_eq!(event.dimension, Dim::End.id());
        assert_eq!(event.cause, TravelCause::EndPortal.id());
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
        if event.cause != TravelCause::EndPortal.id() {
            return Flow::Continue;
        }
        assert_eq!(event.player, u128::from(GUEST));
        assert_eq!(event.from_level, Dim::Overworld.id());
        assert_eq!(event.to_level, Dim::End.id());
        let rider = host.player(event.player).unwrap().entity();
        assert_eq!(rider.position(host), Some(event.from));
        assert_eq!(rider.vehicle(host), None);
        let [x, y, z] = event.from;
        assert_eq!(
            event.to,
            passenger_destination(ENTRY, END_SPAWN, Vec3::new(x, y, z)).components()
        );
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
            assert_eq!(event.to_level, Dim::End.id());
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
fn natural_end_boat_admits_the_guest_and_mob_separately_in_passenger_order() {
    run_on_guest_stack(|| {
        for (refuse_guest, refuse_mob, nested) in [
            (true, false, false),
            (false, true, false),
            (false, false, false),
            (true, true, true),
        ] {
            let steps = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "mixed_portal_policy",
                Box::new(DestinationPolicy {
                    refuse_guest,
                    refuse_mob,
                    steps: steps.clone(),
                }),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    let source = game.dim;
                    assert_eq!(source, Dim::Overworld);
                    let entry = BlockPos::new(3, 70, 5);
                    prepare_entity_ticking_neighborhood(game, ChunkPos::new(0, 0));
                    game.tickets.add_portal((0, 0));
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(entry.x, entry.z));
                    let registry = &game.pack.assets.registry;
                    let portal = registry.default_state("minecraft:end_portal").unwrap();
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
                    game.mobs.push(Mob::new(PIG, species, ENTRY, 10.0));
                    let pig = game.entity(PIG).unwrap();
                    let hull = game.entity(HULL).unwrap();
                    let pig_uuid = game.entity_uuid(pig).unwrap();
                    assert!(game.entity_start_riding(pig, hull, false));
                    if nested {
                        let species = game.res.mob_types()["minecraft:chicken"].clone();
                        game.mobs.push(Mob::new(CHILD, species, ENTRY, 0.0));
                        let child = game.entity(CHILD).unwrap();
                        assert!(game.entity_start_riding(child, pig, true));
                    }
                    game.lan.as_mut().unwrap().hub.update_guest(GUEST, |guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    });
                    let rider = game.player(u128::from(GUEST)).unwrap().entity();
                    assert!(game.entity_start_riding(rider, hull, true));
                    game.guest_boarded(GUEST, None);
                    assert_eq!(game.native_passenger_ids(HULL), [GUEST, PIG]);
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
                    assert_eq!(game.portal_hull_trips.len(), 1);
                    assert!(game.on_server_trip(GUEST));
                    assert!(game.vehicle_waits_in(source));
                    assert_eq!(game.native_passenger_ids(HULL), [GUEST, PIG]);
                    assert!(steps.lock().unwrap().is_empty());

                    game.enter_level(Dim::End).unwrap();
                    game.arrive_server_trips();
                    assert_eq!(game.portal_hull_trips.len(), 1);
                    assert!(game.entity(HULL).is_none());
                    assert!(game.entity(PIG).is_none());
                    assert!(steps.lock().unwrap().is_empty());
                    game.enter_level(source).unwrap();
                    assert_eq!(game.native_passenger_ids(HULL), [GUEST, PIG]);

                    game.enter_level(Dim::End).unwrap();
                    let (cx, cz) = Game::chunk_of(END_SPAWN);
                    for z in cz - 2..=cz + 2 {
                        for x in cx - 2..=cx + 2 {
                            game.streaming
                                .insert_remote(&mut game.world, Chunk::empty(x, z));
                        }
                    }
                    game.arrive_server_trips();
                    assert!(game.portal_hull_trips.is_empty());
                    assert_eq!(game.portal_hull_pose(HULL), Some((END_SPAWN, 90.0)));
                    assert_eq!(game.boats.iter().filter(|boat| boat.id == HULL).count(), 1);
                    let mut expected = vec![
                        Step::Travel(HULL),
                        Step::Dismount(PIG),
                        Step::Dismount(GUEST),
                        Step::Travel(GUEST),
                        Step::OfferGuest,
                    ];
                    if !refuse_guest {
                        expected.push(Step::CommitGuest);
                    }
                    expected.push(Step::Travel(PIG));
                    let mut passengers = Vec::new();
                    for (id, refused) in [(GUEST, refuse_guest), (PIG, refuse_mob)] {
                        if !refused {
                            expected.push(Step::Mount(id));
                            passengers.push(id);
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
                    if refuse_guest {
                        assert_eq!(guest.dimension, source.id());
                        assert_eq!(guest.vehicle, None);
                    } else {
                        assert_eq!(guest.dimension, Dim::End.id());
                        assert_eq!(guest.vehicle, Some(HULL));
                    }

                    game.enter_level(source).unwrap();
                    assert!(game.portal_hull_pose(HULL).is_none());
                    if refuse_mob {
                        let pig = game.entity(PIG).unwrap();
                        assert_eq!(game.entity_uuid(pig), Some(pig_uuid));
                        assert_eq!(game.mob_object(PIG).unwrap().extra.mount, None);
                        if nested {
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
