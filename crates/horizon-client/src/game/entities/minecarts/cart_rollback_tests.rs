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
