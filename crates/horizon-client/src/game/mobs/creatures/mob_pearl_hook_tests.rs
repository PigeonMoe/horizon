use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{
    EnderPearlTeleportEvent, EntityTeleportEvent, Flow, Hooks, ModEntry, ServerHost,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

struct PearlProbe {
    mode: Arc<AtomicU8>,
    owners: Arc<Mutex<Vec<u32>>>,
    command_teleports: Arc<Mutex<Vec<u32>>>,
}
#[mod_api::__subscriptions]
impl Hooks for PearlProbe {
    fn entity_teleport(&self, _: &mut dyn ServerHost, event: &mut EntityTeleportEvent<'_>) {
        self.command_teleports.lock().unwrap().push(event.entity);
    }

    fn ender_pearl_teleport(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EnderPearlTeleportEvent<'_>,
    ) -> Flow {
        self.owners.lock().unwrap().push(event.owner.id());
        assert_eq!(event.owner.position(host), Some(event.from));
        assert_eq!(
            event.damage, 0.0,
            "non-player owners take no vanilla fall damage"
        );
        match self.mode.load(Ordering::SeqCst) {
            0 => Flow::Cancel,
            1 => {
                *event.target = [12.0, 80.0, -4.0];
                Flow::Continue
            }
            2 => {
                assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None));
                *event.target = [12.0, 80.0, -4.0];
                Flow::Continue
            }
            3 => {
                assert!(event.owner.discard(host));
                Flow::Continue
            }
            _ => unreachable!(),
        }
    }
}

struct PlayerPearlProbe(Arc<AtomicU8>);
#[mod_api::__subscriptions]
impl Hooks for PlayerPearlProbe {
    fn ender_pearl_teleport(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EnderPearlTeleportEvent<'_>,
    ) -> Flow {
        assert_eq!(
            host.players().acting_player().unwrap().entity(),
            event.owner
        );
        assert_eq!(event.damage, 5.0);
        // What the pearl hit: the block and its face, no entity.
        assert_eq!(event.hit_entity, None);
        assert_eq!(
            (event.hit_block, event.hit_face, event.hit_at),
            ([6, 75, 6], [0, 1, 0], [6.0, 75.0, 6.0])
        );
        match self.0.load(Ordering::SeqCst) {
            0 => return Flow::Cancel,
            2 => assert!(event.owner.teleport_to(host, [3.0, 70.0, 3.0], None)),
            _ => {}
        }
        *event.target = [12.0, 80.0, -4.0];
        event.damage = 0.0;
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn player_pearl_keeps_acting_context_cancellation_and_mutable_damage() {
    use horizon_client::motion::PlayerMotion;
    let mode = Arc::new(AtomicU8::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "player-pearl-proof",
        Box::new(PlayerPearlProbe(Arc::clone(&mode))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let from = Vec3::new(0.0, 65.0, 0.0);
            game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
            game.lan_acting = Some(12);
            game.lan_acting_uuid = Some(7);
            crate::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
            game.lan_acting_name = Some("PearlTest".into());
            game.lan_acting_permission = Some(4);
            game.lan_acting_entity = Some(2);
            crate::game::native_entity_refs::tests::bind_acting_guest(game);
            game.ui.difficulty = 0;
            let pearl = Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::new(6.0, 75.0, 6.0),
                Some(Who::Player),
            );
            let hit = || Hit::Block {
                pos: [6, 75, 6],
                at: pearl.position,
                face: [0, 1, 0],
            };
            let mut expected_random = game.world.random.clone();
            let health = game.player.as_ref().unwrap().health.health();
            game.projectile_hit(&pearl, hit()).unwrap();
            assert_eq!(game.player.as_ref().unwrap().position, from);
            assert_eq!(game.player.as_ref().unwrap().health.health(), health);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(1, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            expected_random.next_float();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position, Vec3::new(12.0, 80.0, -4.0));
            assert_eq!(
                player.health.health(),
                health,
                "listener suppressed player fall damage"
            );
            assert_eq!(player.fall_distance, 0.0);
            assert_eq!(game.world.random.next_long(), expected_random.next_long());
            mode.store(2, Ordering::SeqCst);
            expected_random = game.world.random.clone();
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(
                player.position,
                Vec3::new(3.0, 70.0, 3.0),
                "a listener's direct move is not overwritten by the pearl target"
            );
            assert_eq!(player.health.health(), health);
            assert_eq!(
                game.world.random.next_long(),
                expected_random.next_long(),
                "a stale player move makes no endermite draw"
            );
        })
    });
}

#[test]
#[ignore = "daily"]
fn mob_pearls_use_the_hook_and_recheck_listener_writes_without_player_side_effects() {
    let mode = Arc::new(AtomicU8::new(0));
    let owners = Arc::new(Mutex::new(Vec::new()));
    let command_teleports = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "mob-pearl-proof",
        Box::new(PearlProbe {
            mode: Arc::clone(&mode),
            owners: Arc::clone(&owners),
            command_teleports: Arc::clone(&command_teleports),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            for action in 0..4 {
                mode.store(action, Ordering::SeqCst);
                let id = 40 + u32::from(action);
                let from = Vec3::new(0.0, 65.0, 0.0);
                let ty = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                let mut mob = Mob::new(id, ty, from, 37.0);
                mob.head_pitch = -12.0;
                mob.previous_yaw = -10.0;
                mob.previous_head_pitch = 20.0;
                mob.velocity = Vec3::new(0.5, -0.25, 0.75);
                mob.fall_distance = 9.0;
                let health = mob.health;
                game.mobs.push(mob);
                let mut pearl = Projectile::new(
                    ProjectileKind::EnderPearl,
                    Vec3::new(6.0, 75.0, 6.0),
                    Some(Who::Mob(id)),
                );
                pearl.id = 100 + id;
                let mut expected_random = game.world.random.clone();
                game.projectile_hit(
                    &pearl,
                    Hit::Block {
                        pos: [6, 75, 6],
                        at: pearl.position,
                        face: [0; 3],
                    },
                )
                .unwrap();
                assert_eq!(
                    game.world.random.next_long(),
                    expected_random.next_long(),
                    "mob pearl handling makes no player endermite draw"
                );
                let mob = game.mobs.iter().find(|m| m.id == id && !m.removed);
                match action {
                    0 => {
                        let m = mob.unwrap();
                        assert_eq!(m.position, from);
                        assert_eq!(m.fall_distance, 9.0);
                    }
                    1 => {
                        let m = mob.unwrap();
                        assert_eq!(m.position, Vec3::new(12.0, 80.0, -4.0));
                        assert_eq!(m.velocity, Vec3::ZERO);
                        assert_eq!(m.fall_distance, 0.0);
                        assert_eq!((m.yaw, m.head_pitch), (37.0, -12.0));
                        assert_eq!((m.previous_yaw, m.previous_head_pitch), (37.0, -12.0));
                        assert_eq!(m.health, health);
                    }
                    2 => assert_eq!(mob.unwrap().position, Vec3::new(3.0, 70.0, 3.0)),
                    3 => assert!(mob.is_none()),
                    _ => unreachable!(),
                }
                assert!(!game.mobs.iter().any(|m| m.species() == Species::Endermite));
                assert_eq!(
                    *command_teleports.lock().unwrap(),
                    if action < 2 { vec![] } else { vec![42] },
                    "only the listener's explicit teleport emits the command notification"
                );
            }
            assert_eq!(*owners.lock().unwrap(), [40, 41, 42, 43]);
        })
    });
}

struct FootwearProbe(Arc<Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl Hooks for FootwearProbe {
    fn player_footwear_reaction(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::PlayerFootwearReactionEvent<'_>,
    ) -> Flow {
        assert_eq!(event.actor, (7, mod_api::GameMode::Survival));
        assert_eq!(
            event.boots.item(),
            mod_api::__private::item_key("minecraft:leather_boots").unwrap()
        );
        assert_eq!(
            event
                .persisted
                .and_then(|data| data.get(&"probe:unlocked".into())),
            Some(&mod_api::Tag::Byte(1))
        );
        assert!(
            !event
                .persisted
                .unwrap()
                .contains_key(&"probe:hidden".into())
        );
        self.0.lock().unwrap().push(event.action.into());
        if event.action == "fall_damage" {
            *event.sound = Some("minecraft:block.slime_block.fall".into());
        }
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn footwear_checks_current_gear_before_farmland_and_pearls_write() {
    use horizon_client::motion::{MotionEvent, MotionWorld, PlayerMotion};
    use horizon_world::fall_damage::FallDamageWorld;
    use mod_api::{Compound, Tag};
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "player-footwear-proof",
        Box::new(FootwearProbe(Arc::clone(&calls))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::entry::install_client_hooks();
        with_game(|game| {
            let from = Vec3::new(6.5, 65.0, 6.5);
            game.player = Some(PlayerMotion::new(game.motion, from).unwrap());
            game.lan_acting = Some(12);
            game.mode = mod_api::GameMode::Survival;
            game.lan_acting_uuid = Some(7);
            crate::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
            game.lan_acting_name = Some("FootwearTest".into());
            game.lan_acting_permission = Some(4);
            game.lan_acting_entity = Some(2);
            crate::game::native_entity_refs::tests::bind_acting_guest(game);
            game.armor[3] = Some(ItemStack {
                item: "minecraft:leather_boots".into(),
                count: 1,
                damage: 0,
                extra: None,
            });
            game.level.player_extra = Some(Tag::Compound(Compound::from([(
                "NeoForgeData".into(),
                Tag::compound([
                    ("probe:hidden", Tag::Byte(1)),
                    (
                        "PlayerPersisted",
                        Tag::compound([("probe:unlocked", Tag::Byte(1))]),
                    ),
                ]),
            )])));
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let farmland = game
                .pack
                .assets
                .registry
                .default_state("minecraft:farmland")
                .unwrap();
            game.world.set(6, 64, 6, farmland).unwrap();
            let health = game.player.as_ref().unwrap().health.health();
            let mut w = MotionWorld {
                landing_equipment: acting_landing_equipment!(game, false),
                entities: &[],
                player: game.player.as_mut().unwrap(),
                scene: &mut game.world,
                assets: &game.pack.assets,
                collision_binding: game.collision,
                resources: game.motion,
                health: &game.health_resources,
                tag_items: Some(world_items!(game)),
                loop_context: &game.loop_context,
                damage_hooks: &game.damage_hooks,
                block_registries: Some(&game.pack.block_registries),
            };
            w.fall_on(farmland, BlockPos::new(6, 64, 6), 8.0).unwrap();
            assert_eq!(w.scene.block(6, 64, 6), farmland);
            assert_eq!(w.player.health.health(), health);
            assert!(w.player.events.iter().any(|event| matches!(
                event,
                MotionEvent::Fall(horizon_client::landing::FallEvent::FootwearSound(
                    "minecraft:block.slime_block.fall"
                ))
            )));

            let pearl = Projectile::new(
                ProjectileKind::EnderPearl,
                Vec3::new(12.0, 75.0, 6.0),
                Some(Who::Player),
            );
            let hit = || Hit::Block {
                pos: [12, 75, 6],
                at: pearl.position,
                face: [0, 1, 0],
            };
            game.projectile_hit(&pearl, hit()).unwrap();
            let player = game.player.as_ref().unwrap();
            assert_eq!(player.position, pearl.previous);
            assert_eq!(player.health.health(), health);
            assert_eq!(player.health.invulnerable_time, 0);
            assert_eq!(
                &*calls.lock().unwrap(),
                &["farmland_trample", "fall_damage", "ender_pearl_damage"]
            );

            // The next action in the same tick uses the actual slot again.
            game.armor[3] = None;
            game.projectile_hit(&pearl, hit()).unwrap();
            assert!(game.player.as_ref().unwrap().health.health() < health);
            assert_eq!(calls.lock().unwrap().len(), 3);
        })
    });
}

struct FlightProbe(Arc<Mutex<Vec<String>>>);

#[mod_api::__subscriptions]
impl Hooks for FlightProbe {
    fn player_gliding_impact(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::PlayerGlidingImpactEvent<'_>,
    ) -> Flow {
        assert_eq!(event.actor, (7, mod_api::GameMode::Survival));
        assert_eq!(
            event.helmet.item(),
            mod_api::__private::item_key("minecraft:leather_helmet").unwrap()
        );
        self.0.lock().unwrap().push(event.action.to_owned());
        *event.reaction = (Some("minecraft:block.stone.hit".into()), true);
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn gliding_impact_wears_the_real_head_slot_and_only_protects_that_glide() {
    use horizon_client::{
        landing::FallEvent,
        motion::{MotionEvent, MotionWorld, PlayerMotion},
    };
    use horizon_world::fall_damage::FallDamageWorld;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "player-flight-impact-proof",
        Box::new(FlightProbe(Arc::clone(&calls))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::entry::install_client_hooks();
        with_game(|game| {
            game.mode = mod_api::GameMode::Survival;
            game.lan_acting_uuid = Some(7);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            for y in 64..=65 {
                for z in 5..=7 {
                    game.world.set(9, y, z, stone).unwrap();
                }
            }
            let start = Vec3::new(8.0, 64.0, 6.5);
            let mut player = PlayerMotion::new(game.motion, start).unwrap();
            let health = player.health.health();
            player.fall_flying = true;
            player.yaw = -90.0;
            player.carry_client_movement(Vec3::new(1.2, 0.0, 0.0));
            game.player = Some(player);
            let mut helmet = ItemStack::new("minecraft:leather_helmet", 1);
            let max = world_items!(game).get(&helmet.item).unwrap().max_damage;
            helmet.damage = max - 1;
            game.armor[0] = Some(helmet.clone());
            let world = |game: &mut Game<'_>, just_stopped_gliding, collide_with_wall| {
                let mut w = MotionWorld {
                    landing_equipment: acting_landing_equipment!(game, just_stopped_gliding),
                    entities: &[],
                    player: game.player.as_mut().unwrap(),
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: Some(world_items!(game)),
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: Some(&game.pack.block_registries),
                };
                w.player.events.clear();
                if collide_with_wall {
                    w.server_travel().unwrap();
                } else {
                    w.player.fall_flying = false;
                    w.fall_on(stone, BlockPos::new(8, 63, 6), 8.0).unwrap();
                }
                let sounds: Vec<_> = w
                    .player
                    .events
                    .iter()
                    .filter_map(|event| {
                        if let MotionEvent::Fall(FallEvent::HelmetSound { name, .. }) = event {
                            Some(*name)
                        } else {
                            None
                        }
                    })
                    .collect();
                (w.player.health.health(), sounds)
            };
            assert_eq!(
                world(game, false, true),
                (health, vec!["minecraft:block.stone.hit"])
            );
            assert_eq!(game.armor[0].as_ref().unwrap().damage, max);
            assert_eq!(
                world(game, true, false),
                (
                    health,
                    vec!["minecraft:block.stone.hit", "minecraft:entity.item.break"]
                )
            );
            assert!(game.armor[0].is_none());
            game.armor[0] = Some(helmet);
            let (health_after, sounds) = world(game, false, false);
            assert!(health_after < health);
            assert!(sounds.is_empty());
            assert_eq!(game.armor[0].as_ref().unwrap().damage, max - 1);
            assert_eq!(&*calls.lock().unwrap(), &["fly_into_wall", "fall_damage"]);
        });
    });
}
