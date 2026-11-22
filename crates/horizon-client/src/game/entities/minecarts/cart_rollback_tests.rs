use super::*;
use crate::game::native_entity_refs::tests::run_on_guest_stack;
use crate::game::native_guest_turn_tests::with_guests;
use horizon_core::position::ChunkPos;
use mod_api::{
    EntityTeleportPreEvent, Flow, Hooks, Host, ModEntry, PlayerTeleportPreEvent,
    RiddenVehicleMovePreEvent, ServerHost,
};
use std::sync::Mutex;

const CART: u32 = 900;
const MOUNT: u32 = 901;
const FIRST_RIDER: u128 = 13;
const SECOND_RIDER: u128 = 12;
const START: Vec3 = Vec3::new(7.875, 64.0625, 8.5);
const REDIRECTED: Vec3 = Vec3::new(6.5, 64.0625, 8.25);
const REDIRECTED_ROTATION: [f32; 2] = [51.0, 12.0];

#[derive(Clone, Copy, Debug)]
enum Policy {
    MoveAllowed,
    OrdinaryVeto,
    RollbackAllowed,
    RollbackRedirected,
    RollbackVeto,
    RecoveryVeto,
}

#[derive(Debug, PartialEq)]
enum Attempt {
    Move(u128),
    CartReturn,
    RiderReturn(u128),
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
        assert_eq!(event.vehicle, CART);
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.from, START.components());
        assert!(event.to[0] >= 8.0);
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::Move(event.player));
        match self.policy {
            Policy::MoveAllowed => Flow::Continue,
            Policy::OrdinaryVeto => Flow::Cancel,
            _ => {
                assert!(!event.paper_rollback);
                event.paper_rollback = true;
                Flow::Cancel
            }
        }
    }

    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(event.entity, CART);
        assert_eq!(event.cause, "vehicle_rollback");
        assert_eq!(event.dimension, "minecraft:overworld");
        assert!(event.from[0] >= 8.0);
        assert_eq!(*event.to, START.components());
        self.attempts.lock().unwrap().push(Attempt::CartReturn);
        match self.policy {
            Policy::RollbackAllowed => Flow::Continue,
            Policy::RollbackRedirected => {
                *event.to = REDIRECTED.components();
                *event.to_rotation = REDIRECTED_ROTATION;
                Flow::Continue
            }
            Policy::RollbackVeto | Policy::RecoveryVeto => Flow::Cancel,
            _ => panic!("an ordinary move should not attempt a cart teleport"),
        }
    }

    fn player_teleport_pre(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerTeleportPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.from_level, "minecraft:overworld");
        assert_eq!(event.to_level, event.from_level);
        assert_eq!(event.to, START.components());
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::RiderReturn(event.player));
        match self.policy {
            Policy::RollbackVeto => Flow::Continue,
            Policy::RecoveryVeto => Flow::Cancel,
            _ => panic!("the cart rollback should not attempt a rider teleport"),
        }
    }
}

fn lay_rails(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    let registry = game.pack.assets.registry;
    let stone = registry.default_state("minecraft:stone").unwrap();
    let rail = crate::game::native_hooks::with_property(
        registry,
        registry.default_state("minecraft:rail").unwrap(),
        "shape",
        "east_west",
    )
    .unwrap();
    for x in 6..=10 {
        game.world.set(x, 63, 8, stone).unwrap();
        game.world.set(x, 64, 8, rail).unwrap();
    }
    let mut cart = Minecart::new(CART, CartKind::Rideable, START);
    cart.velocity.x = 0.4;
    game.carts.push(cart);
}

#[test]
#[ignore = "daily"]
fn sneaking_off_a_minecart_checks_the_landing_after_detaching() {
    check_guest_dismount(false);
}

#[test]
#[ignore = "daily"]
fn powered_activator_rail_checks_the_landing_at_ejection() {
    check_guest_dismount(true);
}

fn check_guest_dismount(powered_rail: bool) {
    use mod_api::PlayerPositionCommittedEvent;

    #[derive(Debug, PartialEq)]
    struct Move(u128, [f64; 3], [f64; 3]);

    #[derive(Default)]
    struct Landing {
        proposed: Mutex<Vec<Move>>,
        committed: Mutex<Vec<Move>>,
    }

    struct Protection(Arc<Landing>);
    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn ridden_vehicle_move_pre(
            &self,
            _: &dyn Host,
            event: &mut RiddenVehicleMovePreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.vehicle, CART);
            event.paper_rollback = true;
            Flow::Cancel
        }

        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            assert_eq!(event.from_level, "minecraft:overworld");
            assert_eq!(event.to_level, event.from_level);
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.vehicle(host), None);
            assert_eq!(rider.position(host), Some(event.from));
            self.0
                .proposed
                .lock()
                .unwrap()
                .push(Move(event.player, event.from, event.to));
            if event.player == FIRST_RIDER {
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
                .committed
                .lock()
                .unwrap()
                .push(Move(event.player, event.from, event.to));
        }
    }

    run_on_guest_stack(move || {
        let landing = Arc::new(Landing::default());
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "minecart-dismount",
                Box::new(Protection(landing.clone())),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    lay_rails(game);
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    game.world.set(7, 63, 7, stone).unwrap();
                    let (input, target) = if powered_rail {
                        let activator = registry.default_state("minecraft:activator_rail").unwrap();
                        let east_west = crate::game::native_hooks::with_property(
                            registry,
                            activator,
                            "shape",
                            "east_west",
                        )
                        .unwrap();
                        let powered = crate::game::native_hooks::with_property(
                            registry, east_west, "powered", "true",
                        )
                        .unwrap();
                        game.world.set(7, 64, 8, powered).unwrap();
                        game.world.set(8, 63, 9, stone).unwrap();
                        // The cart still faces north at ejection, so the east
                        // landing wins. Using its later east-facing yaw would
                        // choose the southern platform instead.
                        (0, [9.5, 64.0, 8.5])
                    } else {
                        (32, [7.5, 64.0, 7.5])
                    };
                    let cart = game.entity(CART).unwrap();
                    let velocity = Vec3::new(0.25, 0.0, 0.0);
                    for uuid in [FIRST_RIDER, SECOND_RIDER] {
                        if powered_rail {
                            game.carts.update(0, |cart| {
                                cart.position = START;
                                cart.yaw = 90.0;
                                cart.flipped = false;
                                cart.velocity = Vec3::new(0.4, 0.0, 0.0);
                            });
                        }
                        let conn = uuid as u32;
                        let rider = game.player(uuid).unwrap().entity();
                        assert!(game.entity_start_riding(rider, cart, true));
                        game.guest_boarded(conn, None);
                        game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.awaiting_teleport = None;
                            guest.motion.as_mut().unwrap().velocity = velocity;
                        });
                        game.lan_player_input(conn, input, true);
                        let guest = game.command_guest(conn).unwrap();
                        assert_eq!(guest.sneaking, !powered_rail);
                        let (from, view) = (guest.position, [guest.yaw, guest.pitch]);
                        assert_ne!(from.map(f64::floor), target.map(f64::floor));
                        game.guest_vehicle_tick(conn);

                        let allowed = uuid == SECOND_RIDER;
                        let expected = if allowed { target } else { from };
                        let guest = game.command_guest(conn).unwrap();
                        assert_eq!(guest.vehicle, None);
                        assert_eq!(guest.position, expected);
                        assert_eq!(guest.awaiting_teleport.unwrap().1, expected);
                        assert_eq!(guest.awaiting_keeps_motion, allowed);
                        assert_eq!([guest.yaw, guest.pitch], view);
                        assert_eq!(
                            guest.motion.as_ref().unwrap().velocity,
                            if allowed { velocity } else { Vec3::ZERO }
                        );
                        if powered_rail {
                            assert!(game.carts[0].position.x >= 8.0);
                            assert_eq!(game.carts[0].yaw, 180.0);
                        } else {
                            assert_eq!(game.carts[0].position, START);
                        }
                        assert_eq!(
                            std::mem::take(&mut *landing.proposed.lock().unwrap()),
                            [Move(uuid, from, target)]
                        );
                        let committed = std::mem::take(&mut *landing.committed.lock().unwrap());
                        if allowed {
                            assert_eq!(committed, [Move(uuid, from, target)]);
                        } else {
                            assert!(committed.is_empty());
                        }
                    }
                });
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn minecart_dismount_chooses_floor_height_and_pose_before_admission() {
    use horizon_client::pose::Pose;

    struct Landing(Arc<Mutex<Vec<[f64; 3]>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Landing {
        fn player_teleport_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            self.0.lock().unwrap().push(event.to);
            Flow::Continue
        }
    }

    run_on_guest_stack(|| {
        let proposed = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "minecart-landing-geometry",
                Box::new(Landing(proposed.clone())),
            )]
            .into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    lay_rails(game);
                    let registry = game.pack.assets.registry;
                    let air = registry.default_state("minecraft:air").unwrap();
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let slab = crate::game::native_hooks::with_property(
                        registry,
                        registry.default_state("minecraft:stone_slab").unwrap(),
                        "type",
                        "bottom",
                    )
                    .unwrap();
                    for x in [6, 8, 9, 10] {
                        game.world.set(x, 63, 8, air).unwrap();
                        game.world.set(x, 64, 8, air).unwrap();
                    }
                    // The preferred side has an unsupported bottom slab with
                    // exactly 1.5 blocks between its surface and the ceiling.
                    game.world.set(7, 64, 7, slab).unwrap();
                    game.world.set(7, 66, 7, stone).unwrap();
                    let cart = game.entity(CART).unwrap();
                    let rider = game.player(FIRST_RIDER).unwrap().entity();
                    let conn = FIRST_RIDER as u32;
                    let dismount = |game: &mut Game<'_>, expected: [f64; 3], pose: Pose| {
                        assert!(game.entity_start_riding(rider, cart, true));
                        game.guest_boarded(conn, None);
                        game.lan_player_input(conn, 32, true);
                        game.guest_vehicle_tick(conn);
                        let guest = game.command_guest(conn).unwrap();
                        assert_eq!(guest.vehicle, None);
                        assert_eq!(guest.position, expected);
                        assert_eq!(guest.awaiting_teleport.unwrap().1, expected);
                        assert_eq!(guest.pose(), pose);
                        assert_eq!(std::mem::take(&mut *proposed.lock().unwrap()), [expected]);
                    };
                    dismount(game, [7.5, 64.5, 7.5], Pose::Crouching);

                    // Try all standing positions before offering a crouching
                    // position closer to the preferred side.
                    game.world.set(7, 63, 9, stone).unwrap();
                    dismount(game, [7.5, 64.0, 9.5], Pose::Standing);

                    // With no side floor, the rider lands on top of the cart.
                    // Its own column now fits only the swimming pose.
                    game.world.set(7, 64, 7, air).unwrap();
                    game.world.set(7, 66, 7, air).unwrap();
                    game.world.set(7, 63, 9, air).unwrap();
                    game.world.set(7, 66, 8, stone).unwrap();
                    let above_cart = [START.x, game.carts[0].bounds().max_position().y, START.z];
                    dismount(game, above_cart, Pose::Swimming);
                });
            },
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_requested_cart_rollback_keeps_ordinary_vetoes_and_checks_rider_recovery() {
    run_on_guest_stack(|| {
        for policy in [
            Policy::MoveAllowed,
            Policy::OrdinaryVeto,
            Policy::RollbackAllowed,
            Policy::RollbackRedirected,
            Policy::RollbackVeto,
            Policy::RecoveryVeto,
        ] {
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let mods = [ModEntry::new(
                "cart-rollback",
                Box::new(Boundary {
                    policy,
                    attempts: attempts.clone(),
                }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_guests(|game| {
                    lay_rails(game);
                    let rider = game.player(FIRST_RIDER).unwrap().entity();
                    let cart = game.entity(CART).unwrap();
                    assert!(game.entity_start_riding(rider, cart, true));
                    game.guest_vehicle_tick(FIRST_RIDER as u32);

                    let mut expected = vec![Attempt::Move(FIRST_RIDER)];
                    let at = game.carts[0].position;
                    match policy {
                        Policy::MoveAllowed => {
                            assert!(at.x >= 8.0);
                            assert_eq!(game.entity_vehicle(rider), Some(cart));
                        }
                        Policy::OrdinaryVeto | Policy::RollbackAllowed => {
                            if matches!(policy, Policy::RollbackAllowed) {
                                expected.push(Attempt::CartReturn);
                            }
                            assert_eq!(at, START);
                            assert_eq!(game.carts[0].velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(rider), Some(cart));
                        }
                        Policy::RollbackRedirected => {
                            expected.push(Attempt::CartReturn);
                            let cart_state = &game.carts[0];
                            assert_eq!(at, REDIRECTED);
                            assert_eq!([cart_state.yaw, cart_state.pitch], REDIRECTED_ROTATION);
                            assert_eq!(cart_state.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(rider), Some(cart));
                            let position = game.entity_position(rider).unwrap();
                            assert_eq!([position[0], position[2]], [REDIRECTED.x, REDIRECTED.z]);
                        }
                        Policy::RollbackVeto | Policy::RecoveryVeto => {
                            expected
                                .extend([Attempt::CartReturn, Attempt::RiderReturn(FIRST_RIDER)]);
                            assert!(at.x >= 8.0);
                            assert_eq!(game.carts[0].velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(rider), None);
                            let position = game.entity_position(rider).unwrap();
                            assert_eq!(
                                position == START.components(),
                                matches!(policy, Policy::RollbackVeto)
                            );
                        }
                    }
                    assert_eq!(*attempts.lock().unwrap(), expected, "{policy:?}");
                });
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_failed_cart_rollback_recovers_the_first_denied_nested_rider_in_boarding_order() {
    run_on_guest_stack(|| {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "cart-ordered-rollback",
            Box::new(Boundary {
                policy: Policy::RollbackVeto,
                attempts: attempts.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                lay_rails(game);
                let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                game.mobs.push(Mob::new(MOUNT, kind, START, 0.0));
                let cart = game.entity(CART).unwrap();
                let mount = game.entity(MOUNT).unwrap();
                let first = game.player(FIRST_RIDER).unwrap().entity();
                let second = game.player(SECOND_RIDER).unwrap().entity();
                assert!(game.entity_start_riding(mount, cart, true));
                assert!(game.entity_start_riding(first, mount, true));
                assert!(game.entity_start_riding(second, mount, true));
                assert_eq!(
                    game.entity_indirect_passengers(cart),
                    Some(vec![mount, first, second])
                );

                game.tick_carts();
                assert_eq!(
                    *attempts.lock().unwrap(),
                    [
                        Attempt::Move(FIRST_RIDER),
                        Attempt::CartReturn,
                        Attempt::RiderReturn(FIRST_RIDER)
                    ]
                );
                assert!(game.carts[0].position.x >= 8.0);
                assert_eq!(game.carts[0].velocity, Vec3::ZERO);
                assert_eq!(game.entity_vehicle(mount), None);
                assert_eq!(game.entity_vehicle(first), None);
                assert_eq!(game.entity_position(first), Some(START.components()));
                assert_eq!(game.entity_vehicle(second), Some(mount));
            });
        });
    });
}
