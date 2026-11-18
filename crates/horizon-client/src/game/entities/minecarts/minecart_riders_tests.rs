//! `AbstractMinecart`'s rider seat and dismount side: a villager or wandering
//! trader sits at the cart's own y, and the old behavior sets a rider down
//! beside its motion direction, not beside its facing.
use super::*;

/// `AbstractMinecart.getPassengerAttachmentPoint` lowers a villager's and a
/// wandering trader's seat to the cart's own y; any other rider keeps the
/// cart's 0.1875 attachment.
#[test]
fn a_villager_sits_on_the_cart_itself() {
    let at = Vec3::new(1.0, 10.0, 2.0);
    for species in [Species::Villager, Species::WanderingTrader] {
        assert_eq!(cart_attachment(species), 0.0, "{species:?}");
        // Neither has a `ridingOffset` of its own.
        let seat = cart_seat(at, cart_seat_offset(species, 0.0));
        assert!(
            (seat.y - 10.0).abs() < 1e-12,
            "{species:?} sits at {seat:?}"
        );
    }
    // A cow keeps the cart's 0.1875, less a `ridingOffset` if it had one.
    let cow = cart_seat(at, cart_seat_offset(Species::Cow, 0.0));
    assert!((cow.y - 10.1875).abs() < 1e-12, "a cow sits at {cow:?}");
    assert_eq!(cart_seat_offset(Species::Cow, 0.7), PASSENGER_Y - 0.7);
    // A zombie villager is no `Villager`.
    assert_eq!(cart_attachment(Species::ZombieVillager), PASSENGER_Y);
}

/// `AbstractMinecart.getDismountLocationForPassenger` starts the search from
/// `getMotionDirection`, and `DismountHelper.offsetsForDirection` tries the
/// right of that direction first.
#[test]
fn an_eastbound_cart_sets_its_rider_down_on_its_south_side() {
    // `OldMinecartBehavior.tick`'s yaw of a cart moving east, flipped or not;
    // its `getMotionDirection` turns that facing clockwise, to east.
    for (yaw, flipped) in [(180.0, false), (0.0, true)] {
        let face = cart_motion_face(yaw, flipped, false);
        assert_eq!(
            dismount_offsets(face)[0],
            (0, 1),
            "yaw {yaw}, flipped {flipped}"
        );
    }
    // `MinecartBehavior.getMotionDirection` under the experiment is the
    // facing itself, so the first candidate stays east of a north-facing cart.
    let face = cart_motion_face(180.0, false, true);
    assert_eq!(dismount_offsets(face)[0], (1, 0));
}

#[test]
#[ignore = "daily"]
fn a_denied_rail_crossing_stops_the_cart_and_preserves_its_seat() {
    use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
    use mod_api::{Flow, Hooks, Host, ModEntry, Mods, RiddenVehicleMovePreEvent};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Boundary {
        allow: bool,
        crossings: usize,
    }
    struct Guard(Arc<Mutex<Boundary>>);
    #[mod_api::__subscriptions]
    impl Hooks for Guard {
        fn ridden_vehicle_move_pre(
            &self,
            _: &dyn Host,
            event: &mut RiddenVehicleMovePreEvent<'_>,
        ) -> Flow {
            if event.from[0] >= 8.0 || event.to[0] < 8.0 {
                return Flow::Continue;
            }
            assert_eq!(event.player, 0x81);
            assert_eq!(event.vehicle, 900);
            let mut boundary = self.0.lock().unwrap();
            boundary.crossings += 1;
            if boundary.allow {
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }
    }

    let boundary = Arc::new(Mutex::new(Boundary::default()));
    let mods: Mods = vec![ModEntry::new(
        "cart-boundary",
        Box::new(Guard(boundary.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let registry = game.pack.assets.registry;
            let stone = registry.default_state("minecraft:stone").unwrap();
            let rail = registry.default_state("minecraft:rail").unwrap();
            let rail =
                crate::game::native_hooks::with_property(registry, rail, "shape", "east_west")
                    .unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            for x in 6..=10 {
                game.world.set(x, 63, 8, stone).unwrap();
                game.world.set(x, 64, 8, rail).unwrap();
            }
            let from = Vec3::new(7.875, 64.0625, 8.5);
            let seat = Vec3::new(from.x, from.y + SEAT, from.z);
            let mut cart = Minecart::new(900, CartKind::Rideable, from);
            cart.velocity.x = 0.4;
            game.carts.push(cart);
            game.player =
                Some(horizon_client::motion::PlayerMotion::new(game.motion, seat).unwrap());
            bind_local_player(game, 901, 0x81);
            game.riding = Some(900);

            assert!(game.tick_cart_riding(Keys::default()));
            assert_eq!(boundary.lock().unwrap().crossings, 1);
            assert_eq!(game.carts[0].position, from);
            assert_eq!(game.carts[0].velocity, Vec3::ZERO);
            assert_eq!(game.riding, Some(900));
            assert_eq!(game.player.as_ref().unwrap().position, seat);

            boundary.lock().unwrap().allow = true;
            game.carts.update(0, |cart| cart.velocity.x = 0.4);
            assert!(game.tick_cart_riding(Keys::default()));
            assert_eq!(boundary.lock().unwrap().crossings, 2);
            assert_eq!(game.riding, Some(900));
            let at = game.carts[0].position;
            assert!(at.x >= 8.0);
            assert_eq!(
                game.player.as_ref().unwrap().position,
                Vec3::new(at.x, at.y + SEAT, at.z)
            );
        });
    });
}
