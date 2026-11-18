//! Production residency regressions for projectile ownership.
#![cfg(test)]
use crate::game::Game;
use horizon_client::{
    motion::PlayerMotion,
    projectiles::{Projectile, ProjectileKind},
};
use horizon_core::geometry::Vec3;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::{Arc, Mutex};

fn arrow(id: u32, at: [f64; 3]) -> Projectile {
    let mut p = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::new(at[0], at[1], at[2]),
        None,
    );
    p.id = id;
    p
}

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "projectile_restore",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
#[ignore = "daily"]
fn projectile_restore_entries_keep_the_bound_arrow() {
    for path in 0..3 {
        let exercise = |game: &mut Game<'_>| {
            let at = [3.0, 64.0, 5.0];
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
                .unwrap();
            game.creatures.projectiles.push(arrow(71, at));
            game.creatures
                .projectiles
                .update(0, |p| p.velocity = Vec3::new(0.3, 0.0, 0.0));
            let index = Arc::new(Mutex::new(AttachmentIndex::default()));
            let mut values = Attachments::default();
            VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
            let original = game.creatures.projectiles.update(0, |p| {
                p.attachment_lifetime().bind(&mut values, &index, holder);
                p.attachment_lifetime().registration().unwrap()
            });
            let key = ("test".into(), "projectile_restore".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 1);
            if path == 0 {
                game.tick_remote_projectiles(|_, at| at);
                assert!(game.creatures.projectiles[0].position.x > at[0]);
            } else {
                // One takeable unbound arrow exercises the pickup branch while
                // the bound, disallowed arrow must survive the same retain.
                let mut picked = arrow(72, at);
                let state = picked.arrow.as_mut().unwrap();
                state.in_ground = true;
                state.pickup = horizon_client::arrows::Pickup::Allowed;
                for id in [73, 74] {
                    game.creatures.projectiles.push(arrow(id, at));
                }
                game.creatures.projectiles.push(picked);
                if path == 1 {
                    crate::game::entity::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.guest_arrows(0);
                        },
                    );
                } else {
                    game.player = Some(
                        PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).unwrap(),
                    );
                    crate::game::entity::taken_pass_tests::assert_projectile_spawn_survives(
                        game,
                        |game| {
                            game.touch_arrows();
                        },
                    );
                }
                assert_eq!(
                    game.creatures
                        .projectiles
                        .iter()
                        .map(|p| p.id)
                        .collect::<Vec<_>>(),
                    [71, 73, 74]
                );
            }
            let current = game
                .creatures
                .projectiles
                .update(0, |p| p.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current), "restore path {path}");
            assert_eq!(
                index.lock().unwrap().snapshot(&key).len(),
                1,
                "restore path {path}"
            );
            game.creatures.projectiles.clear();
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        };
        if path == 0 {
            crate::game::native_entity_refs::tests::with_client_game(exercise);
        } else {
            crate::game::native_guest_turn_tests::with_guests(exercise);
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_guest_arrow_pickup_veto_keeps_both_arrow_and_inventory() {
    use mod_api::{Flow, Hooks, ModEntry, PlayerPickupArrowPreEvent, ServerHost};
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Veto(Arc<AtomicBool>);
    #[mod_api::__subscriptions]
    impl Hooks for Veto {
        fn player_pickup_arrow_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerPickupArrowPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.player, 12);
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.position, [3.75, 64.0, 5.0]);
            assert_eq!(event.item, "minecraft:arrow");
            if self.0.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    let canceled = Arc::new(AtomicBool::new(true));
    let mods = vec![ModEntry::new(
        "arrow_probe",
        Box::new(Veto(canceled.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            let mut shot = arrow(71, [3.75, 64.0, 5.0]);
            let state = shot.arrow.as_mut().unwrap();
            state.in_ground = true;
            state.pickup = horizon_client::arrows::Pickup::Allowed;
            game.creatures.projectiles.push(shot);
            let arrow_count = |game: &Game<'_>| {
                game.lan
                    .as_ref()
                    .unwrap()
                    .hub
                    .guest(12)
                    .unwrap()
                    .inventory
                    .iter()
                    .flatten()
                    .filter(|stack| stack.item.as_ref() == "minecraft:arrow")
                    .map(|stack| stack.count)
                    .sum::<u32>()
            };
            game.guest_arrows(0);
            assert_eq!(game.creatures.projectiles.len(), 1);
            assert_eq!(arrow_count(game), 0);

            canceled.store(false, Ordering::Relaxed);
            game.guest_arrows(0);
            assert!(game.creatures.projectiles.is_empty());
            assert_eq!(arrow_count(game), 1);
        });
    });
}
