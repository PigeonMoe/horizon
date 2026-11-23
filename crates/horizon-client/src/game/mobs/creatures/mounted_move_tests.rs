use super::*;
use crate::game::{
    native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
};
use horizon_client::{chunk::Chunk, entities::ArmorStand};
use horizon_core::position::ChunkPos;
use mod_api::{Flow, Hooks, Host, ModEntry, RiddenVehicleMovePreEvent};
use std::sync::atomic::{AtomicUsize, Ordering};

const MOUNT: u32 = 151;
const STAND: u32 = 152;
const RIDER: u128 = 13;
const START: Vec3 = Vec3::new(7.875, 200.0, 8.5);

struct Boundary(Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl Hooks for Boundary {
    fn ridden_vehicle_move_pre(
        &self,
        _: &dyn Host,
        event: &mut RiddenVehicleMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.player, RIDER);
        assert_eq!(event.vehicle, MOUNT);
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.from, START.components());
        assert!(event.to[0] >= 8.0);
        if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

fn check_living_mount_crossing(nested: bool) {
    run_on_guest_stack(move || {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = [ModEntry::new(
            "living-mount-boundary",
            Box::new(Boundary(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                assert!(game.run_command("forceload add 0 0"));

                let kind = if nested {
                    "minecraft:horse"
                } else {
                    "minecraft:pig"
                };
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                let mut mount = Mob::new(MOUNT, kind, START, 0.0);
                mount.no_gravity = true;
                game.mobs.push(mount);
                let rider = game.player(RIDER).unwrap().entity();
                let seat = if nested {
                    game.armor_stands.push(ArmorStand::new(STAND, START, 0.0));
                    game.link_loaded_passenger(STAND, MOUNT);
                    assert_eq!(game.passenger_vehicle(STAND), Some(MOUNT));
                    STAND
                } else {
                    MOUNT
                };
                let seat = game.entity(seat).unwrap();
                assert!(game.entity_start_riding(rider, seat, true));
                game.mobs[0].velocity.x = 0.5;

                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert_eq!(game.mobs[0].position, START);
                assert!(game.mobs[0].velocity.x > 0.0);
                assert_eq!(game.entity_vehicle(rider), Some(seat));

                // A second tick crosses under the remaining momentum alone.
                game.tick_creature_entities().unwrap();
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                assert!(game.mobs[0].position.x >= 8.0);
                assert_eq!(game.entity_vehicle(rider), Some(seat));
                if nested {
                    assert_eq!(game.passenger_vehicle(STAND), Some(MOUNT));
                }
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_denied_living_mount_crossing_preserves_momentum_and_seat() {
    check_living_mount_crossing(false);
}

#[test]
#[ignore = "daily"]
fn a_living_mount_checks_a_player_riding_its_armor_stand_passenger() {
    check_living_mount_crossing(true);
}
