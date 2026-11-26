use super::*;
use crate::game::{
    native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
};
use horizon_client::{
    chunk::Chunk,
    lan::{Out, PlayerSender},
    mobs::Mob,
};
use horizon_core::position::ChunkPos;
use horizon_protocol::{
    buf::Reader,
    packets::play::{Clientbound, MoveVehicleRequest},
};
use mod_api::{
    EntityTeleportPreEvent, Flow, Hooks, Host, ModEntry, PlayerTeleportPreEvent,
    RiddenVehicleMovePreEvent, ServerHost,
};
use std::sync::{Mutex, mpsc};

const BOAT: u32 = 900;
const CAMEL: u32 = 901;
const FIRST: u128 = 13;
const SECOND: u128 = 12;
const START: Vec3 = Vec3::new(7.875, 200.0, 8.5);
const CROSSING_OFFSET: f64 = 1.3;
const REDIRECTED: Vec3 = Vec3::new(5.625, 200.0, 8.75);
const REDIRECTED_ROTATION: [f32; 2] = [37.0, -8.0];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Policy {
    Allow,
    Stop,
    RecoverHull,
    RedirectHull,
    RecoverRider,
    DenyRiderReturn,
}

#[derive(Debug, PartialEq)]
enum Attempt {
    Move(u128),
    HullTeleport,
    RiderTeleport(u128),
}

struct Boundary {
    policy: Policy,
    attempts: Arc<Mutex<Vec<Attempt>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Boundary {
    fn ridden_vehicle_move_pre(
        &self,
        _: &dyn Host,
        event: &mut RiddenVehicleMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.vehicle, BOAT);
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.from, START.components());
        assert!(event.to[0] >= 8.0);
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::Move(event.player));
        match self.policy {
            Policy::Allow => Flow::Continue,
            Policy::Stop => Flow::Cancel,
            _ => {
                event.paper_rollback = true;
                Flow::Cancel
            }
        }
    }

    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(event.entity, BOAT);
        assert_eq!(event.cause, "vehicle_rollback");
        assert_eq!(event.dimension, "minecraft:overworld");
        assert!(event.from[0] >= 8.0);
        assert_eq!(*event.to, START.components());
        self.attempts.lock().unwrap().push(Attempt::HullTeleport);
        match self.policy {
            Policy::RecoverHull => Flow::Continue,
            Policy::RedirectHull => {
                *event.to = REDIRECTED.components();
                *event.to_rotation = REDIRECTED_ROTATION;
                Flow::Continue
            }
            Policy::RecoverRider | Policy::DenyRiderReturn => Flow::Cancel,
            _ => panic!("the policy didn't request a hull teleport"),
        }
    }

    fn player_teleport_pre(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerTeleportPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.to, START.components());
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::RiderTeleport(event.player));
        match self.policy {
            Policy::RecoverRider => Flow::Continue,
            Policy::DenyRiderReturn => Flow::Cancel,
            _ => panic!("the hull did not need to eject a rider"),
        }
    }
}

fn launch(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    let mut boat = Boat::new(BOAT, "oak".into(), START, 20.0);
    boat.no_gravity = true;
    boat.velocity.x = 0.5;
    game.boats.push(boat);
}

fn received_packets(packets: &mpsc::Receiver<Out>) -> Vec<Clientbound> {
    packets
        .try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            _ => Vec::new(),
        })
        .filter_map(|body| {
            let mut reader = Reader::new(&body);
            let id = reader.varint().ok()?;
            Clientbound::decode(id, reader.rest()).ok()
        })
        .collect()
}

#[test]
#[ignore = "daily"]
fn a_boat_dismount_teleport_can_be_denied_without_keeping_the_seat() {
    check_boat_dismount(false);
}

#[test]
#[ignore = "daily"]
fn a_boat_in_a_portal_checks_its_free_dismount_landing() {
    check_boat_dismount(true);
}

fn check_boat_dismount(portal: bool) {
    use horizon_client::lan::guest::InboxItem;
    use horizon_protocol::packets::play::{PlayerInput, Serverbound, relative};
    use mod_api::PlayerPositionCommittedEvent;

    #[derive(Debug, PartialEq)]
    enum Notice {
        Proposed(u128, [f64; 3], [f64; 3]),
        Committed(u128, [f64; 3], [f64; 3]),
    }

    struct Landing(Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Landing {
        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            assert_eq!(
                (event.from_level, event.to_level),
                ("minecraft:overworld", "minecraft:overworld")
            );
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.vehicle(host), None);
            assert_eq!(rider.position(host), Some(event.from));
            self.0
                .lock()
                .unwrap()
                .push(Notice::Proposed(event.player, event.from, event.to));
            if event.player == FIRST {
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
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.vehicle(host), None);
            assert_eq!(rider.position(host), Some(event.to));
            self.0
                .lock()
                .unwrap()
                .push(Notice::Committed(event.player, event.from, event.to));
        }
    }

    run_on_guest_stack(move || {
        let seen = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "dismount_region",
                Box::new(Landing(seen.clone())),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    launch(game);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(7, 199, 9, stone).unwrap();
                    if portal {
                        let portal = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:nether_portal")
                            .unwrap();
                        game.world.set(7, 200, 8, portal).unwrap();
                    }
                    let hull = game.entity(BOAT).unwrap();
                    let velocity = Vec3::new(0.25, 0.0, 0.0);
                    for uuid in [FIRST, SECOND] {
                        let conn = uuid as u32;
                        let rider = game.player(uuid).unwrap().entity();
                        assert!(game.entity_start_riding(rider, hull, true));
                        game.guest_boarded(conn, None);
                        let (sender, packets) = mpsc::channel();
                        game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                            guest.sender = PlayerSender::new(sender, Arc::default());
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.awaiting_teleport = None;
                            guest.motion.as_mut().unwrap().velocity = velocity;
                            guest.inbox.push_back(InboxItem::Packet(
                                Serverbound::PlayerInput(PlayerInput { flags: 32 }),
                                None,
                            ));
                        });
                        game.level_pass.packets_drained = false;
                        game.tick_lan_packets();
                        let guest = game.command_guest(conn).unwrap();
                        assert!(guest.sneaking, "player {uuid} did not receive sneak input");
                        let from = guest.position;
                        let target = if portal {
                            [from[0], START.y, from[2]]
                        } else {
                            game.boat_player_dismount_target(0, guest.yaw).components()
                        };
                        assert_ne!(from.map(f64::floor), target.map(f64::floor));
                        game.guest_vehicle_tick(conn);

                        let expected = if uuid == FIRST { from } else { target };
                        let guest = game.command_guest(conn).unwrap();
                        assert_eq!(guest.vehicle, None);
                        assert_eq!(guest.position, expected);
                        assert_eq!(guest.awaiting_teleport.unwrap().1, expected);
                        let (momentum, relatives) = if uuid == FIRST {
                            (Vec3::ZERO, 0)
                        } else {
                            (
                                velocity,
                                relative::DELTA_X
                                    | relative::DELTA_Y
                                    | relative::DELTA_Z
                                    | relative::ROTATE_DELTA
                                    | relative::X_ROT
                                    | relative::Y_ROT,
                            )
                        };
                        assert_eq!(guest.motion.as_ref().unwrap().velocity, momentum);
                        let messages = received_packets(&packets);
                        let updates: Vec<_> = messages
                            .iter()
                            .filter(|packet| {
                                matches!(
                                    packet,
                                    Clientbound::SetPassengers(_) | Clientbound::PlayerPosition(_)
                                )
                            })
                            .collect();
                        assert!(matches!(
                            updates.as_slice(),
                            [Clientbound::SetPassengers(seats), Clientbound::PlayerPosition(at)]
                            if seats.vehicle == BOAT as i32 && seats.passengers.is_empty()
                                && at.position == expected && at.relatives == relatives
                        ));
                        let events = std::mem::take(&mut *seen.lock().unwrap());
                        let mut expected_notices = vec![Notice::Proposed(uuid, from, target)];
                        if uuid == SECOND {
                            expected_notices.push(Notice::Committed(uuid, from, target));
                        }
                        assert_eq!(events, expected_notices);
                    }
                });
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn command_and_native_live_dismounts_check_the_edge_and_then_the_landing() {
    use mod_api::{EntityMountEvent, PlayerPositionCommittedEvent};
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Debug, PartialEq)]
    enum Notice {
        Edge(u32),
        Landing(u128, [f64; 3], [f64; 3]),
        Committed(u128, [f64; 3], [f64; 3]),
    }

    #[derive(Default)]
    struct Policy {
        veto_edge: AtomicBool,
        notices: Mutex<Vec<Notice>>,
    }
    struct Protection(Arc<Policy>);
    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            if event.mounting {
                return Flow::Continue;
            }
            assert_eq!(event.vehicle, BOAT);
            self.0
                .notices
                .lock()
                .unwrap()
                .push(Notice::Edge(event.entity));
            if self.0.veto_edge.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.is_passenger(host), Some(false));
            assert_eq!(rider.position(host), Some(event.from));
            self.0.notices.lock().unwrap().push(Notice::Landing(
                event.player,
                event.from,
                event.to,
            ));
            if event.player == FIRST {
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
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.is_passenger(host), Some(false));
            assert_eq!(rider.position(host), Some(event.to));
            self.0.notices.lock().unwrap().push(Notice::Committed(
                event.player,
                event.from,
                event.to,
            ));
        }
    }

    run_on_guest_stack(|| {
        let policy = Arc::new(Policy::default());
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "explicit_disembark",
                Box::new(Protection(policy.clone())),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    launch(game);
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(7, 199, 9, stone).unwrap();
                    let command = |game: &mut Game<'_>, player: u128| {
                        game.as_console(|game| {
                            game.run_command(&format!("ride guest{player} dismount"))
                        })
                    };
                    let hull = game.entity(BOAT).unwrap();
                    for player in [FIRST, SECOND] {
                        let conn = player as u32;
                        let rider = game.player(player).unwrap().entity();
                        let edge = Notice::Edge(rider.id());
                        assert!(!command(game, player));
                        assert!(!rider.stop_riding(game));
                        assert!(policy.notices.lock().unwrap().is_empty());
                        assert!(rider.start_riding(game, hull, true));
                        game.guest_boarded(conn, None);
                        let (sender, packets) = mpsc::channel();
                        game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                            guest.sender = PlayerSender::new(sender, Arc::default());
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.awaiting_teleport = None;
                        });
                        let guest = game.command_guest(conn).unwrap();
                        let from = guest.position;
                        let target = game.boat_player_dismount_target(0, guest.yaw).components();
                        assert_ne!(from, target);
                        if player == FIRST {
                            policy.veto_edge.store(true, Ordering::Relaxed);
                            // RideCommand sends success after stopRiding even if
                            // a mod kept the original mount edge.
                            assert!(command(game, player));
                            assert!(!rider.stop_riding(game));
                            assert_eq!(game.command_guest(conn).unwrap().vehicle, Some(BOAT));
                            assert_eq!(
                                std::mem::take(&mut *policy.notices.lock().unwrap()),
                                [Notice::Edge(rider.id()), Notice::Edge(rider.id())]
                            );
                            assert!(!received_packets(&packets).iter().any(|p| matches!(
                                p,
                                Clientbound::SetPassengers(_) | Clientbound::PlayerPosition(_)
                            )));
                            policy.veto_edge.store(false, Ordering::Relaxed);
                            assert!(
                                rider.stop_riding(game),
                                "a landing veto still detached the rider"
                            );
                        } else {
                            assert!(command(game, player));
                        }
                        let guest = game.command_guest(conn).unwrap();
                        let at = if player == FIRST { from } else { target };
                        assert_eq!(guest.position, at);
                        assert_eq!(guest.vehicle, None);
                        let updates: Vec<_> = received_packets(&packets)
                            .into_iter()
                            .filter(|p| {
                                matches!(
                                    p,
                                    Clientbound::SetPassengers(_) | Clientbound::PlayerPosition(_)
                                )
                            })
                            .collect();
                        assert!(matches!(
                            updates.as_slice(),
                            [Clientbound::SetPassengers(seats), Clientbound::PlayerPosition(position)]
                            if seats.vehicle == BOAT as i32 && seats.passengers.is_empty() && position.position == at
                        ));
                        let mut expected = vec![edge, Notice::Landing(player, from, target)];
                        if player == SECOND {
                            expected.push(Notice::Committed(player, from, target));
                        }
                        assert_eq!(
                            std::mem::take(&mut *policy.notices.lock().unwrap()),
                            expected
                        );
                    }
                })
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn destroyed_boat_detaches_the_acting_rider_even_if_the_landing_is_denied() {
    check_removed_vehicle_dismount(RemovedDismountCase::DeniedBoat);
}

#[test]
#[ignore = "daily"]
fn destroyed_cart_retries_a_denied_edge_before_teleporting_its_rider() {
    check_removed_vehicle_dismount(RemovedDismountCase::RetriedCart);
}

#[test]
#[ignore = "daily"]
fn discarded_boat_does_not_override_a_later_teleport_in_the_guest_turn() {
    check_removed_vehicle_dismount(RemovedDismountCase::SupersededBoat);
}

#[derive(Clone, Copy, PartialEq)]
enum RemovedDismountCase {
    DeniedBoat,
    RetriedCart,
    SupersededBoat,
}

fn check_removed_vehicle_dismount(case: RemovedDismountCase) {
    use crate::game::{
        commands::Target,
        minecarts::{CartHit, CartKind, Minecart},
    };
    use horizon_protocol::packets::play::relative;
    use mod_api::{EntityMountEvent, PlayerPositionCommittedEvent};

    #[derive(Debug, PartialEq)]
    enum Notice {
        LeftSeat,
        Proposed([f64; 3], [f64; 3]),
        Committed([f64; 3], [f64; 3]),
    }

    struct Protection(RemovedDismountCase, Arc<Mutex<Vec<Notice>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn entity_mount(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityMountEvent<'_>,
        ) -> Flow {
            if event.mounting {
                return Flow::Continue;
            }
            assert_eq!(event.vehicle, BOAT);
            assert!(host.entity(BOAT).is_none());
            let rider = host.player(FIRST).unwrap().entity();
            assert_eq!(event.entity, rider.id());
            assert_eq!(rider.is_passenger(host), Some(true));
            let mut seen = self.1.lock().unwrap();
            let retry = self.0 == RemovedDismountCase::RetriedCart && seen.is_empty();
            seen.push(Notice::LeftSeat);
            if retry { Flow::Cancel } else { Flow::Continue }
        }

        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            assert_eq!(event.player, FIRST);
            let rider = host.player(FIRST).unwrap().entity();
            assert_eq!(rider.is_passenger(host), Some(false));
            assert_eq!(rider.position(host), Some(event.from));
            self.1
                .lock()
                .unwrap()
                .push(Notice::Proposed(event.from, event.to));
            if self.0 == RemovedDismountCase::DeniedBoat {
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
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.is_passenger(host), Some(false));
            assert_eq!(rider.position(host), Some(event.to));
            self.1
                .lock()
                .unwrap()
                .push(Notice::Committed(event.from, event.to));
        }
    }

    run_on_guest_stack(move || {
        let seen = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "removed_vehicle_region",
                Box::new(Protection(case, seen.clone())),
            )]
            .into(),
            || {
                with_guests(|game| {
                    let conn = FIRST as u32;
                    game.options.dedicated = Some(Vec::new());
                    launch(game);
                    if case == RemovedDismountCase::RetriedCart {
                        game.boats.clear();
                        game.carts
                            .push(Minecart::new(BOAT, CartKind::Rideable, START));
                    }
                    let vehicle = game.entity(BOAT).unwrap();
                    let rider = game.player(FIRST).unwrap().entity();
                    assert!(game.entity_start_riding(rider, vehicle, true));
                    game.guest_boarded(conn, None);
                    let (sender, packets) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                        guest.sender = PlayerSender::new(sender, Arc::default());
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.awaiting_teleport = None;
                    });
                    let from = game.command_guest(conn).unwrap().position;
                    let landing = [from[0], START.y, from[2]];
                    let replacement = [3.0, 201.0, 5.0];
                    assert!(from[1] < START.y);
                    match case {
                        RemovedDismountCase::DeniedBoat => {
                            game.as_guest(conn, |game| assert!(game.hurt_boat(0, 5.0, false)));
                        }
                        RemovedDismountCase::RetriedCart => {
                            game.hurt_cart(0, 5.0, false, false, CartHit::default());
                            assert!(game.persist.removed_vehicles.contains_key(&BOAT));
                            assert_eq!(game.command_guest(conn).unwrap().vehicle, Some(BOAT));
                            assert_eq!(&*seen.lock().unwrap(), &[Notice::LeftSeat]);
                            assert!(!received_packets(&packets).iter().any(|packet| matches!(
                                packet,
                                Clientbound::SetPassengers(_) | Clientbound::PlayerPosition(_)
                            )));
                            game.guest_vehicle_tick(conn);
                        }
                        RemovedDismountCase::SupersededBoat => {
                            game.as_guest(conn, |game| {
                                assert!(vehicle.discard(game));
                                game.teleport_to_in_level(
                                    Target::Player,
                                    replacement,
                                    None,
                                    [false; 3],
                                    false,
                                );
                            });
                            game.native_removals_flush();
                        }
                    }
                    assert!(game.entity(BOAT).is_none());
                    assert!(!game.persist.removed_vehicles.contains_key(&BOAT));
                    let (at, flags, mut expected) = match case {
                        RemovedDismountCase::DeniedBoat => (from, 0, vec![Notice::LeftSeat]),
                        RemovedDismountCase::RetriedCart => (
                            landing,
                            relative::DELTA_X
                                | relative::DELTA_Y
                                | relative::DELTA_Z
                                | relative::ROTATE_DELTA
                                | relative::X_ROT
                                | relative::Y_ROT,
                            vec![Notice::LeftSeat, Notice::LeftSeat],
                        ),
                        RemovedDismountCase::SupersededBoat => {
                            (replacement, 0, vec![Notice::LeftSeat])
                        }
                    };
                    expected.push(Notice::Proposed(from, landing));
                    if at != from {
                        expected.push(Notice::Committed(from, at));
                    }
                    assert_eq!(*seen.lock().unwrap(), expected);
                    let guest = game.command_guest(conn).unwrap();
                    assert_eq!(guest.vehicle, None);
                    assert_eq!(guest.vehicle_good, None);
                    assert_eq!(guest.position, at);
                    assert_eq!(guest.motion.as_ref().unwrap().position.components(), at);
                    let id = guest.awaiting_teleport.unwrap().0;
                    let updates: Vec<_> = received_packets(&packets)
                        .into_iter()
                        .filter(|packet| {
                            matches!(
                                packet,
                                Clientbound::SetPassengers(_) | Clientbound::PlayerPosition(_)
                            )
                        })
                        .collect();
                    assert!(matches!(
                        updates.as_slice(),
                        [Clientbound::SetPassengers(seats), Clientbound::PlayerPosition(position)]
                        if seats.vehicle == BOAT as i32 && seats.passengers.is_empty()
                            && position.teleport == id && position.position == at && position.relatives == flags
                    ));
                    game.guest_vehicle_tick(conn);
                    assert_eq!(
                        game.command_guest(conn).unwrap().awaiting_teleport,
                        Some((id, at))
                    );
                    assert_eq!(*seen.lock().unwrap(), expected);
                });
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_server_driven_boat_checks_the_hull_return_and_recovers_only_the_denied_rider() {
    run_on_guest_stack(|| {
        for policy in [
            Policy::Allow,
            Policy::Stop,
            Policy::RecoverHull,
            Policy::RedirectHull,
            Policy::RecoverRider,
            Policy::DenyRiderReturn,
        ] {
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let mods = [ModEntry::new(
                "boat-rollback",
                Box::new(Boundary {
                    policy,
                    attempts: attempts.clone(),
                }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_guests(|game| {
                    launch(game);
                    let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                    game.mobs.push(Mob::new(CAMEL, kind, START, 0.0));
                    let boat = game.entity(BOAT).unwrap();
                    let camel = game.entity(CAMEL).unwrap();
                    let first = game.player(FIRST).unwrap().entity();
                    let second = game.player(SECOND).unwrap().entity();
                    assert!(game.entity_start_riding(camel, boat, true));
                    assert!(game.entity_start_riding(first, camel, true));
                    assert!(game.entity_start_riding(second, camel, true));
                    assert_eq!(
                        game.entity_indirect_passengers(boat),
                        Some(vec![camel, first, second])
                    );

                    let previous_positions = (policy == Policy::RedirectHull).then(|| {
                        // Native mounting establishes the links. Seat the chain
                        // as the server would before its next hull tick.
                        game.position_riders();
                        game.guest_boarded(FIRST as u32, None);
                        game.guest_boarded(SECOND as u32, None);
                        [camel, first, second].map(|rider| game.entity_position(rider).unwrap())
                    });
                    // Its direct passenger is a mob, so the server moves the hull.
                    game.tick_boats();
                    let mut expected = vec![Attempt::Move(FIRST)];
                    let hull = &game.boats[0];
                    match policy {
                        Policy::Allow => {
                            expected.push(Attempt::Move(SECOND));
                            assert!(hull.position.x >= 8.0);
                            assert_eq!(game.entity_vehicle(camel), Some(boat));
                            assert_eq!(game.entity_vehicle(first), Some(camel));
                        }
                        Policy::Stop | Policy::RecoverHull => {
                            if policy == Policy::RecoverHull {
                                expected.push(Attempt::HullTeleport);
                            }
                            assert_eq!(hull.position, START);
                            assert_eq!(hull.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(camel), Some(boat));
                            assert_eq!(game.entity_vehicle(first), Some(camel));
                        }
                        Policy::RedirectHull => {
                            expected.push(Attempt::HullTeleport);
                            assert_eq!(hull.position, REDIRECTED);
                            assert_eq!([hull.yaw, hull.pitch], REDIRECTED_ROTATION);
                            assert_eq!(hull.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(camel), Some(boat));
                            assert_eq!(game.entity_vehicle(first), Some(camel));
                            let delta = REDIRECTED.subtract(START).components();
                            for (rider, previous) in [camel, first, second]
                                .into_iter()
                                .zip(previous_positions.unwrap())
                            {
                                let actual = game.entity_position(rider).unwrap();
                                for axis in 0..3 {
                                    assert!(
                                        (actual[axis] - previous[axis] - delta[axis]).abs() < 1e-8,
                                        "seat {rider:?}, from {previous:?} to {actual:?}"
                                    );
                                }
                            }
                        }
                        Policy::RecoverRider | Policy::DenyRiderReturn => {
                            expected.extend([Attempt::HullTeleport, Attempt::RiderTeleport(FIRST)]);
                            assert!(hull.position.x >= 8.0);
                            assert_eq!(hull.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(camel), None);
                            if policy == Policy::RecoverRider {
                                assert_eq!(game.entity_vehicle(first), None);
                                assert_eq!(game.entity_position(first), Some(START.components()));
                            } else {
                                assert_eq!(game.entity_vehicle(first), Some(camel));
                            }
                        }
                    }
                    assert_eq!(game.entity_vehicle(second), Some(camel));
                    assert_eq!(*attempts.lock().unwrap(), expected, "{policy:?}");
                });
            });
        }
    });
}

fn launch_nested_camel_driver(game: &mut Game<'_>) -> mpsc::Receiver<Out> {
    launch(game);
    let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
    let mut camel = Mob::new(CAMEL, kind, START, 0.0);
    camel.extra.set_saddled(true);
    game.mobs.push(camel);
    let boat = game.entity(BOAT).unwrap();
    let mount = game.entity(CAMEL).unwrap();
    let driver = game.player(FIRST).unwrap().entity();
    let conn = FIRST as u32;
    assert!(game.entity_start_riding(mount, boat, true));
    assert!(game.entity_start_riding(driver, mount, true));
    game.position_riders();
    game.guest_boarded(conn, None);
    let (tx, packets) = mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |guest| {
            guest.sender = PlayerSender::new(tx, Arc::default());
            guest.loaded = true;
            guest.load_timeout = 0;
            guest.awaiting_teleport = None;
        })
        .unwrap();

    // The first ride tick installs the client controls; the next one
    // establishes a baseline at the original mount position.
    game.guest_vehicle_tick(conn);
    game.guest_vehicle_tick(conn);
    let mount = game.entity_position(mount).unwrap();
    let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
    assert_eq!(guest.vehicle_good, Some(mount));
    received_packets(&packets);
    packets
}

#[test]
#[ignore = "daily"]
fn a_nested_driver_cannot_move_its_mount_until_it_is_the_root_vehicle() {
    use horizon_client::lan::guest::InboxItem;
    use horizon_protocol::packets::play::{AcceptTeleportation, Serverbound};

    run_on_guest_stack(|| {
        with_guests(|game| {
            let packets = launch_nested_camel_driver(game);
            let mount = game.entity(CAMEL).unwrap();
            let driver = game.player(FIRST).unwrap().entity();
            let conn = FIRST as u32;

            assert!(game.place_vehicle_rollback(
                crate::game::commands::Target::Boat(BOAT),
                REDIRECTED.components(),
                REDIRECTED_ROTATION,
            ));
            let at = game.entity_position(mount).unwrap();
            assert_ne!(
                at,
                REDIRECTED.components(),
                "the camel has its own seat offset"
            );
            let pose = received_packets(&packets)
                .into_iter()
                .filter_map(|packet| match packet {
                    Clientbound::TeleportEntity(packet) if packet.id == CAMEL as i32 => {
                        Some(packet.position)
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(pose, [at]);

            let target = [at[0] + 0.05, at[1], at[2]];
            let mount_state = game.mob_object(CAMEL).unwrap();
            let (yaw, pitch) = (mount_state.yaw, mount_state.head_pitch);
            game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                assert_eq!(guest.vehicle_good, Some(at));
                let (id, [x, y, z]) = guest.awaiting_teleport.unwrap();
                for packet in [
                    Serverbound::AcceptTeleportation(AcceptTeleportation {
                        id,
                        x,
                        y,
                        z,
                        yaw: guest.yaw,
                        pitch: guest.pitch,
                    }),
                    Serverbound::MoveVehicle(MoveVehicleRequest {
                        position: target,
                        yaw,
                        pitch,
                        on_ground: false,
                    }),
                ] {
                    guest.inbox.push_back(InboxItem::Packet(packet, None));
                }
            });
            // The rider confirmed the player teleport, but the camel is
            // still a passenger of the boat and cannot move independently.
            game.tick_lan_packets();
            assert_eq!(game.entity_position(mount), Some(at));
            assert_eq!(game.entity_vehicle(driver), Some(mount));
            assert!(
                received_packets(&packets)
                    .iter()
                    .all(|packet| !matches!(packet, Clientbound::MoveVehicle(_)))
            );

            assert!(game.entity_stop_riding(mount));
            assert_eq!(game.entity_vehicle(mount), None);
            game.guest_vehicle_tick(conn);
            let at = game.entity_position(mount).unwrap();
            let mount_state = game.mob_object(CAMEL).unwrap();
            let target = [at[0] + 0.05, at[1], at[2]];
            game.guest_move_vehicle(
                conn,
                &MoveVehicleRequest {
                    position: target,
                    yaw: mount_state.yaw,
                    pitch: mount_state.head_pitch,
                    on_ground: false,
                },
            );
            assert_eq!(game.entity_position(mount), Some(target));
            assert_eq!(game.entity_vehicle(driver), Some(mount));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_nested_driver_keeps_only_the_player_teleport_for_an_off_level_mount() {
    use crate::game::persist::OffLevel;

    run_on_guest_stack(|| {
        with_guests(|game| {
            let packets = launch_nested_camel_driver(game);
            let conn = FIRST as u32;
            let driver = game.player(FIRST).unwrap().entity();
            let from = game.entity_position(driver).unwrap();
            let index = game.mobs.iter().position(|mob| mob.id == CAMEL).unwrap();
            let mount = game.mobs.remove(index);
            let moved_mount = REDIRECTED.add_vector(mount.position.subtract(START));
            game.persist.off_level.insert(CAMEL, OffLevel::Mob(mount));
            assert!(game.entity(CAMEL).is_none());

            assert!(game.place_vehicle_rollback(
                crate::game::commands::Target::Boat(BOAT),
                REDIRECTED.components(),
                REDIRECTED_ROTATION,
            ));
            assert!(game.entity(CAMEL).is_none());
            let mount = game.mob_object(CAMEL).unwrap();
            assert_eq!(mount.position, moved_mount);
            assert_eq!(mount.extra.mount, Some(BOAT));

            // Keep the edge through placement; cross-level transfer reads it
            // to map the player to the replacement vehicle.
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!(guest.vehicle, Some(CAMEL));
            assert_eq!(guest.vehicle_good, None);
            let at = REDIRECTED.add_vector(Vec3::new(from[0], from[1], from[2]).subtract(START));
            assert_eq!(guest.position, at.components());
            let (_, pending) = guest.awaiting_teleport.unwrap();
            assert_eq!(pending, at.components());
            let messages = received_packets(&packets);
            assert!(messages.iter().any(|message| matches!(
                message,
                Clientbound::PlayerPosition(packet) if packet.position == at.components()
            )));
            assert!(messages.iter().all(|message| !matches!(
                message,
                Clientbound::TeleportEntity(packet) if packet.id == CAMEL as i32
            )));

            game.guest_vehicle_tick(conn);
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert_eq!((guest.vehicle, guest.vehicle_good), (None, None));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_rejected_guest_boat_packet_keeps_its_immediate_correction() {
    run_on_guest_stack(|| {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "boat-packet",
            Box::new(Boundary {
                policy: Policy::RecoverRider,
                attempts: attempts.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                launch(game);
                let rider = game.player(FIRST).unwrap().entity();
                let boat = game.entity(BOAT).unwrap();
                assert!(game.entity_start_riding(rider, boat, true));
                let (tx, packets) = mpsc::channel();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_connection(FIRST as u32, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.awaiting_teleport = None;
                    })
                    .unwrap();

                game.guest_move_vehicle(
                    FIRST as u32,
                    &MoveVehicleRequest {
                        position: [START.x + 0.5, START.y, START.z],
                        yaw: 20.0,
                        pitch: 0.0,
                        on_ground: false,
                    },
                );
                assert_eq!(game.boats[0].position, START);
                assert_eq!(game.boats[0].velocity, Vec3::ZERO);
                assert_eq!(game.entity_vehicle(rider), Some(boat));
                assert_eq!(*attempts.lock().unwrap(), [Attempt::Move(FIRST)]);
                let corrections: Vec<_> = packets
                    .try_iter()
                    .filter_map(|out| {
                        let Out::Packet(body) = out else {
                            return None;
                        };
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let Ok(Clientbound::MoveVehicle(packet)) =
                            Clientbound::decode(id, reader.rest())
                        {
                            Some((packet.position, packet.yaw))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(corrections, [(START.components(), 20.0)]);

                game.tick_boats();
                assert_eq!(game.boats[0].position, START);
                assert_eq!(game.entity_vehicle(rider), Some(boat));
                assert_eq!(*attempts.lock().unwrap(), [Attempt::Move(FIRST)]);
            });
        });
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GuestPolicy {
    DenyDriver,
    LegacyVeto,
    RecoverHull,
    Rewrite,
    InvalidRewrite,
    VetoRewrite,
    Eject,
}

struct GuestBoundary {
    policy: GuestPolicy,
    checks: Arc<Mutex<Vec<u128>>>,
}

#[mod_api::__subscriptions]
impl Hooks for GuestBoundary {
    fn ridden_vehicle_move_pre(
        &self,
        _: &dyn Host,
        event: &mut RiddenVehicleMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.guest_driver, Some(FIRST));
        self.checks.lock().unwrap().push(event.player);
        if (self.policy == GuestPolicy::DenyDriver && event.player == FIRST)
            || (self.policy == GuestPolicy::LegacyVeto && event.player == SECOND)
        {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn guest_boat_tick_move_pre(
        &self,
        _: &dyn Host,
        event: &mut mod_api::GuestBoatTickMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.vehicle, BOAT);
        assert_eq!(event.from, START.components());
        assert_eq!(event.to, [START.x + CROSSING_OFFSET, START.y, START.z]);
        self.checks.lock().unwrap().push(event.player + 100);
        if event.player == SECOND {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(event.entity, BOAT);
        assert_eq!(*event.to, START.components());
        self.checks.lock().unwrap().push(200);
        if self.policy == GuestPolicy::Eject {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn player_teleport_pre(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerTeleportPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.player, SECOND);
        assert_eq!(event.to, START.components());
        self.checks.lock().unwrap().push(300);
        Flow::Continue
    }
}

struct HullDestination {
    policy: GuestPolicy,
    checks: Arc<Mutex<Vec<u128>>>,
}

#[derive(Debug, PartialEq)]
enum DriverPolicyEvent {
    Packet(Option<[f64; 3]>),
    BoatTick,
    Rollback,
}

struct PaperDriverBoundary(Arc<Mutex<Vec<DriverPolicyEvent>>>);

#[mod_api::__subscriptions]
impl Hooks for PaperDriverBoundary {
    fn ridden_vehicle_move_pre(
        &self,
        _: &dyn Host,
        event: &mut RiddenVehicleMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.guest_driver, Some(FIRST));
        assert_eq!(event.player, FIRST);
        self.0
            .lock()
            .unwrap()
            .push(DriverPolicyEvent::Packet(event.driver_move_from));
        // Movement is denied in the entire area starting at x=8, so a
        // mounted player cannot move to another blocked cell in that area.
        if event.to[0] >= 8.0
            && event
                .driver_move_from
                .is_some_and(|from| from[0].floor() != event.to[0].floor())
        {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn guest_boat_tick_move_pre(
        &self,
        _: &dyn Host,
        event: &mut mod_api::GuestBoatTickMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.vehicle, BOAT);
        assert_eq!(event.player, FIRST);
        assert!(event.from[0] < 8.0 && event.to[0] >= 8.0);
        self.0.lock().unwrap().push(DriverPolicyEvent::BoatTick);
        Flow::Cancel
    }

    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(event.entity, BOAT);
        assert_eq!(event.cause, "vehicle_rollback");
        self.0.lock().unwrap().push(DriverPolicyEvent::Rollback);
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn paper_driver_packets_defer_small_boat_moves_and_check_horses_within_a_claim() {
    run_on_guest_stack(|| {
        for is_boat in [true, false] {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let mods = vec![ModEntry::new(
                "paper-driver-boundary",
                Box::new(PaperDriverBoundary(seen.clone())),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_guests(|game| {
                    launch(game);
                    let (id, x, distance) = if is_boat {
                        (BOAT, 7.984375, 0.03125)
                    } else {
                        (CAMEL, 8.5, 1.0)
                    };
                    let from = Vec3::new(x, START.y, START.z);
                    let to = [x + distance, START.y, START.z];
                    if is_boat {
                        game.boats.update(0, |boat| {
                            boat.position = from;
                            boat.velocity = Vec3::ZERO;
                        });
                    } else {
                        let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                        let mut horse = Mob::new(id, kind, from, 20.0);
                        horse.extra.set_saddled(true);
                        horse.no_gravity = true;
                        game.mobs.push(horse);
                    }
                    let vehicle = game.entity(id).unwrap();
                    let driver = game.player(FIRST).unwrap().entity();
                    let conn = FIRST as u32;
                    assert!(game.entity_start_riding(driver, vehicle, true));
                    game.guest_boarded(conn, None);
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.awaiting_teleport = None;
                    });
                    game.guest_vehicle_tick(conn);
                    game.guest_vehicle_tick(conn);
                    if is_boat {
                        game.tick_boats();
                    }
                    game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                        guest.reset_paper_move_pose(from.components(), [20.0, 0.0]);
                    });
                    assert!(seen.lock().unwrap().is_empty());

                    game.guest_move_vehicle(
                        conn,
                        &MoveVehicleRequest {
                            position: to,
                            yaw: 20.0,
                            pitch: 0.0,
                            on_ground: false,
                        },
                    );

                    if is_boat {
                        assert_eq!(game.entity_position(vehicle), Some(to));
                        assert_eq!(*seen.lock().unwrap(), [DriverPolicyEvent::Packet(None)]);
                        game.tick_boats();
                        assert_eq!(
                            *seen.lock().unwrap(),
                            [
                                DriverPolicyEvent::Packet(None),
                                DriverPolicyEvent::BoatTick,
                                DriverPolicyEvent::Rollback,
                            ]
                        );
                    } else {
                        assert_eq!(
                            *seen.lock().unwrap(),
                            [DriverPolicyEvent::Packet(Some(from.components())),]
                        );
                    }
                    assert_eq!(game.entity_position(vehicle), Some(from.components()));
                    assert_eq!(game.entity_vehicle(driver), Some(vehicle));
                });
            });
        }
    });
}

#[mod_api::__subscriptions]
impl Hooks for HullDestination {
    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(*event.to, START.components());
        self.checks.lock().unwrap().push(201);
        *event.to = REDIRECTED.components();
        *event.to_rotation = REDIRECTED_ROTATION;
        match self.policy {
            GuestPolicy::VetoRewrite => Flow::Cancel,
            GuestPolicy::InvalidRewrite => {
                event.to[0] = f64::NAN;
                Flow::Continue
            }
            _ => Flow::Continue,
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_guest_boat_keeps_immediate_vetoes_and_checks_the_final_hull_once() {
    run_on_guest_stack(|| {
        for policy in [
            GuestPolicy::DenyDriver,
            GuestPolicy::LegacyVeto,
            GuestPolicy::RecoverHull,
            GuestPolicy::Rewrite,
            GuestPolicy::InvalidRewrite,
            GuestPolicy::VetoRewrite,
            GuestPolicy::Eject,
        ] {
            let checks = Arc::new(Mutex::new(Vec::new()));
            let mut mods = vec![ModEntry::new(
                "guest-boat-move-phases",
                Box::new(GuestBoundary {
                    policy,
                    checks: checks.clone(),
                }),
            )];
            let edited = matches!(
                policy,
                GuestPolicy::Rewrite | GuestPolicy::InvalidRewrite | GuestPolicy::VetoRewrite
            );
            if edited {
                mods.push(ModEntry::new(
                    "guest-boat-rollback-destination",
                    Box::new(HullDestination {
                        policy,
                        checks: checks.clone(),
                    }),
                ));
            }
            mod_platform::testing::with_mods(mods.into(), || {
                with_guests(|game| {
                    launch(game);
                    let boat = game.entity(BOAT).unwrap();
                    let first = game.player(FIRST).unwrap().entity();
                    let second = game.player(SECOND).unwrap().entity();
                    assert!(game.entity_start_riding(first, boat, true));
                    assert!(game.entity_start_riding(second, boat, true));
                    let (tx, packets) = mpsc::channel();
                    let mut sender = Some(PlayerSender::new(tx, Arc::default()));
                    game.lan.as_mut().unwrap().hub.guests.update_all(|guest| {
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.awaiting_teleport = None;
                        if guest.uuid == FIRST {
                            guest.sender = sender.take().unwrap();
                        }
                    });
                    let good = |game: &Game<'_>, conn| {
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(conn)
                            .unwrap()
                            .vehicle_good
                    };
                    let request = |game: &mut Game<'_>, dx| {
                        game.guest_move_vehicle(
                            FIRST as u32,
                            &MoveVehicleRequest {
                                position: [START.x + dx, START.y, START.z],
                                yaw: 20.0,
                                pitch: 0.0,
                                on_ground: false,
                            },
                        );
                    };
                    let received = || received_packets(&packets);

                    // Initialize the tick baseline while both guests are seated.
                    game.guest_vehicle_tick(FIRST as u32);
                    game.guest_vehicle_tick(SECOND as u32);
                    game.tick_boats();
                    assert!(checks.lock().unwrap().is_empty());
                    let old_seats =
                        [first, second].map(|rider| game.entity_position(rider).unwrap());
                    received();
                    request(game, 0.3);
                    if matches!(policy, GuestPolicy::DenyDriver | GuestPolicy::LegacyVeto) {
                        let before = checks.lock().unwrap().clone();
                        assert!(before.contains(
                            &(if policy == GuestPolicy::DenyDriver {
                                FIRST
                            } else {
                                SECOND
                            })
                        ));
                        assert_eq!(game.boats[0].position, START);
                        game.tick_boats();
                        assert_eq!(*checks.lock().unwrap(), before);
                        let corrections = received()
                            .into_iter()
                            .filter_map(|p| match p {
                                Clientbound::MoveVehicle(p) => Some(p.position),
                                Clientbound::TeleportEntity(_) => {
                                    panic!("an early veto should not send a hull teleport")
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(corrections, [START.components()]);
                        return;
                    }
                    // The returning rider stays clear of the boat's ordinary contact area.
                    request(game, CROSSING_OFFSET);
                    assert_eq!(game.boats[0].position.x, START.x + CROSSING_OFFSET);
                    assert_eq!(*checks.lock().unwrap(), [SECOND, FIRST, SECOND, FIRST]);
                    game.tick_boats();
                    let mut expected =
                        vec![SECOND, FIRST, SECOND, FIRST, FIRST + 100, SECOND + 100, 200];
                    if edited {
                        expected.push(201);
                    }
                    let messages = received();
                    assert!(
                        messages
                            .iter()
                            .all(|p| !matches!(p, Clientbound::MoveVehicle(_)))
                    );
                    if matches!(policy, GuestPolicy::RecoverHull | GuestPolicy::Rewrite) {
                        let (destination, rotation) = if policy == GuestPolicy::Rewrite {
                            (REDIRECTED, REDIRECTED_ROTATION)
                        } else {
                            (START, [20.0, 0.0])
                        };
                        assert_eq!(game.boats[0].position, destination);
                        assert_eq!([game.boats[0].yaw, game.boats[0].pitch], rotation);
                        for (conn, rider, before) in
                            [(FIRST, first, old_seats[0]), (SECOND, second, old_seats[1])]
                        {
                            assert_eq!(good(game, conn as u32), Some(destination.components()));
                            assert_eq!(game.entity_vehicle(rider), Some(boat));
                            let at = game.entity_position(rider).unwrap();
                            assert!((at[0] - before[0] - (destination.x - START.x)).abs() < 1e-8);
                        }
                        let saved = messages
                            .iter()
                            .filter_map(|p| match p {
                                Clientbound::TeleportEntity(p) if p.id == BOAT as i32 => {
                                    Some((p.position, [p.yaw, p.pitch]))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        assert_eq!(saved, [(destination.components(), rotation)]);
                        let driver = game.lan.as_ref().unwrap().hub.guest(FIRST as u32).unwrap();
                        assert!(driver.awaiting_teleport.is_some());
                    } else {
                        expected.push(300);
                        assert_eq!(game.boats[0].position.x, START.x + CROSSING_OFFSET);
                        for (conn, rider) in [(FIRST, first), (SECOND, second)] {
                            assert_eq!(good(game, conn as u32), None);
                            assert_eq!(game.entity_vehicle(rider), None);
                        }
                        assert_eq!(game.entity_position(second), Some(START.components()));
                    }
                    assert_eq!(game.boats[0].velocity, Vec3::ZERO);
                    game.tick_boats();
                    assert_eq!(*checks.lock().unwrap(), expected, "{policy:?}");

                    if policy == GuestPolicy::RecoverHull {
                        use horizon_client::lan::guest::InboxItem;
                        use horizon_protocol::packets::play::{AcceptTeleportation, Serverbound};
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .guests
                            .update_connection(FIRST as u32, |driver| {
                                let (id, [x, y, z]) = driver.awaiting_teleport.unwrap();
                                driver.inbox.push_back(InboxItem::Packet(
                                    Serverbound::AcceptTeleportation(AcceptTeleportation {
                                        id,
                                        x,
                                        y,
                                        z,
                                        yaw: driver.yaw,
                                        pitch: driver.pitch,
                                    }),
                                    None,
                                ));
                            })
                            .unwrap();
                        game.tick_lan_packets();
                        request(game, 0.05);
                        assert_eq!(game.boats[0].position.x, START.x + 0.05);
                        assert_eq!(
                            good(game, FIRST as u32),
                            Some([START.x + 0.05, START.y, START.z])
                        );
                        assert!(
                            received()
                                .into_iter()
                                .all(|p| !matches!(p, Clientbound::MoveVehicle(_)))
                        );
                    }
                });
            });
        }
    });
}
