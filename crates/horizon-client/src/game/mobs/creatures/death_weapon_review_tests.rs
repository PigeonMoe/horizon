use super::tests::{WeaponProbe, isolated};
use super::*;
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "daily"]
fn llama_spit_uses_the_shooter_for_damage_and_only_a_player_shooter_for_protection() {
    if isolated() {
        return;
    }
    use crate::game::{
        native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
    };
    use horizon_client::lan::playerdata::uuid_ints;
    use mod_api::{
        EntityDamageTargetPreEvent, Flow, Hooks, LevelRandomHost, LivingIncomingDamageEvent,
        ModEntry,
    };

    const OWNER: u32 = 13;
    const ACTIVE_VICTIM: u32 = 12;
    const LLAMA: u32 = 82;
    const WOLF: u32 = 83;
    const UNINVOLVED: u32 = 84;
    const IMPACT: [f64; 3] = [8.5, 200.0, 6.5];

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Case {
        LlamaToWolf,
        LlamaToGuest,
        PlayerToWolf,
    }

    #[derive(Default)]
    struct Observation {
        actors: Mutex<Vec<(String, Option<u128>)>>,
        sources: Mutex<Vec<(i32, String, i32, i32)>>,
    }

    struct Probe(Arc<Observation>);

    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            let player = event.player.map(|player| player.uuid);
            self.0
                .actors
                .lock()
                .unwrap()
                .push((event.target_kind.into(), player));
            if player.is_some() {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn living_incoming_damage(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            let source = event.source;
            self.0.sources.lock().unwrap().push((
                event.entity,
                source.damage_type.into(),
                source.causing,
                source.direct,
            ));
            Flow::Continue
        }
    }

    crate::entry::install_client_hooks();
    run_on_guest_stack(|| {
        for case in [Case::LlamaToWolf, Case::LlamaToGuest, Case::PlayerToWolf] {
            let seen = Arc::new(Observation::default());
            let mods = vec![ModEntry::new("spit_policy", Box::new(Probe(seen.clone())))].into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.as_console(|game| assert!(game.run_command("difficulty normal")));
                    game.load_ticking_neighbourhood(0, 0);
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(8, 6));
                    let wolf_target = case != Case::LlamaToGuest;
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    for (conn, position) in [
                        (OWNER, [32.0, 200.0, 32.0]),
                        (
                            ACTIVE_VICTIM,
                            if wolf_target {
                                [30.0, 200.0, 30.0]
                            } else {
                                IMPACT
                            },
                        ),
                    ] {
                        hub.update_guest(conn, |guest| {
                            guest.dimension = "minecraft:overworld".into();
                            guest.loaded = true;
                            guest.loading = false;
                            guest.load_timeout = 0;
                            guest.position = position;
                            guest.previous_position = position;
                        })
                        .unwrap();
                    }
                    let kind = game.res.mob_types()["minecraft:llama"].clone();
                    let mut llama = Mob::new(LLAMA, kind, Vec3::new(3.5, 200.0, 6.5), 0.0);
                    llama.extra.tame = true;
                    llama.extra.owner = Some(uuid_ints(u128::from(OWNER)));
                    game.mobs.push(llama);
                    let victim = if wolf_target { WOLF } else { ACTIVE_VICTIM };
                    if wolf_target {
                        let wolf = game.res.mob_types()["minecraft:wolf"].clone();
                        game.mobs.push(Mob::new(
                            WOLF,
                            wolf,
                            Vec3::new(IMPACT[0], IMPACT[1], IMPACT[2]),
                            0.0,
                        ));
                    } else {
                        // Closer to the spit than its shooter, outside its forward path.
                        let pig = game.res.mob_types()["minecraft:pig"].clone();
                        game.mobs
                            .push(Mob::new(UNINVOLVED, pig, Vec3::new(5.0, 200.0, 6.5), 0.0));
                    }
                    let health = |game: &Game<'_>| {
                        if wolf_target {
                            game.mobs.iter().find(|mob| mob.id == WOLF).unwrap().health
                        } else {
                            game.lan
                                .as_ref()
                                .unwrap()
                                .hub
                                .guest(ACTIVE_VICTIM)
                                .unwrap()
                                .health
                        }
                    };
                    let before = health(game);
                    let player_shooter = case == Case::PlayerToWolf;
                    let mut spit = Projectile::new(
                        ProjectileKind::LlamaSpit,
                        Vec3::new(6.5, 200.45, 6.5),
                        Some(if player_shooter {
                            Who::Player
                        } else {
                            Who::Mob(LLAMA)
                        }),
                    );
                    if player_shooter {
                        spit.guest = Some(OWNER);
                        spit.owner_uuid = Some(uuid_ints(u128::from(OWNER)));
                    }
                    spit.left_owner = true;
                    spit.no_gravity = true;
                    spit.velocity = Vec3::new(3.0, 0.0, 0.0);
                    let spit_id = game.add_projectile(spit);
                    game.tick_projectiles().unwrap();
                    let kind = if wolf_target {
                        "minecraft:wolf"
                    } else {
                        "minecraft:player"
                    };
                    assert_eq!(
                        *seen.actors.lock().unwrap(),
                        [(kind.into(), player_shooter.then_some(u128::from(OWNER)))],
                        "{case:?}"
                    );
                    let sources = seen.sources.lock().unwrap();
                    if player_shooter {
                        assert_eq!(health(game), before);
                        assert!(sources.is_empty());
                    } else {
                        assert_eq!(health(game), before - 1.0, "{case:?}");
                        assert_eq!(
                            *sources,
                            [(
                                victim as i32,
                                "minecraft:spit".into(),
                                LLAMA as i32,
                                spit_id as i32
                            )],
                            "{case:?}"
                        );
                    }
                });
            });
        }
    });
}

fn fixture(expected: Option<ItemStack>, run: impl FnOnce(&mut Game<'_>)) {
    crate::game::native_entity_refs::tests::with_game(|game| {
        let scratch = tempfile::tempdir().unwrap();
        game.world_root = scratch.path().to_path_buf();
        game.save = Arc::new(horizon_client::save::SaveDir::new(
            scratch.path().join("save"),
            horizon_client::save::LevelShape::OVERWORLD,
        ));
        game.build_registries();
        game.next_entity = 1000;
        let loot = horizon_world::loot::LootData::load(
            &game.options.resources,
            &Default::default(),
            game.res.item_defaults(),
        )
        .unwrap();
        game.server_loot.set(Some(Arc::new(loot)));
        let seen = Arc::new(Mutex::new(0));
        let mods = vec![mod_api::ModEntry::new(
            "review_weapon",
            Box::new(WeaponProbe {
                expected: expected.map(|stack| stack.try_into_api().unwrap()),
                swap: None,
                seen: seen.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || run(game));
        assert_eq!(*seen.lock().unwrap(), 1);
    });
}
fn victim(game: &Game<'_>) -> Mob {
    Mob::new(
        70,
        game.res.mob_types().get("minecraft:pig").unwrap().clone(),
        Vec3::new(2.0, 64.0, 0.0),
        0.0,
    )
}
fn arrow(weapon: Option<ItemStack>) -> Projectile {
    let mut shot = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        Vec3::ZERO,
        None,
    );
    shot.arrow.as_mut().unwrap().fired_from_weapon = weapon;
    shot.id = 912;
    shot
}
fn hit(game: &mut Game<'_>, shot: &Projectile, amount: f32) -> bool {
    let mut damage = Damage::of(DamageKind::Arrow);
    damage.from = Some([shot.position.x, shot.position.y, shot.position.z]);
    game.damage_projectile_with_weapon(shot, 70, amount, damage, None)
}

#[test]
#[ignore = "daily"]
fn death_weapon_immune_crossbow_cannot_replace_accepted_crossbow() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut expected = ItemStack::new("minecraft:crossbow", 1);
        expected.enchant("minecraft:piercing", 1);
        fixture(Some(expected), |game| {
            let mut target = victim(game);
            target.kind = game
                .res
                .mob_types()
                .get("minecraft:sulfur_cube")
                .unwrap()
                .clone();
            game.mobs.push(target);
            let mut first = ItemStack::new("minecraft:crossbow", 1);
            first.enchant("minecraft:piercing", 1);
            assert!(hit(game, &arrow(Some(first)), 1.0));
            let target = game.mobs.iter_mut().find(|m| m.id == 70).unwrap();
            let committed = target.last_damage.unwrap();
            target.extra.set_body(Some("minecraft:stone".into()));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                2.0
            ));
            let mut dead = game.mobs.iter().find(|m| m.id == 70).unwrap().clone();
            assert_eq!(dead.last_damage.unwrap().1, committed.1);
            dead.health = 0.0;
            game.mob_died(&dead);
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_refused_crossbow_cannot_replace_accepted_longbow() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut bow = ItemStack::new("minecraft:bow", 1);
        bow.enchant("minecraft:power", 2);
        bow.data_mut()
            .set_loot_name("minecraft:custom_name", &serde_json::json!("Longbow"));
        fixture(Some(bow.clone()), |game| {
            game.mobs.push(victim(game));
            let first = arrow(Some(bow));
            assert!(hit(game, &first, 100.0));
            let victim = game.mobs.iter().find(|mob| mob.id == 70).unwrap();
            assert_eq!(victim.health, 0.0);
            assert!(victim.invulnerable > 0);
            let committed = victim.last_damage;
            let second = arrow(Some(ItemStack::new("minecraft:crossbow", 1)));
            // Damage calculation must not capture; the subsequent hit is refused.
            let mut attempted = second.clone();
            game.shot_damage(&mut attempted, Some("minecraft:pig"), Who::Mob(70));
            assert!(!hit(game, &second, 1.0));
            let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
            assert_eq!(dead.last_damage.unwrap().1, committed.unwrap().1);
            game.mob_died(&dead);
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_dispenser_arrow_without_recorded_weapon_is_none() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        game.mobs.push(victim(game));
        // First accept a recorded weapon, then accept an unrecorded arrow after cooldown.
        assert!(hit(
            game,
            &arrow(Some(ItemStack::new("minecraft:bow", 1))),
            1.0
        ));
        game.mobs
            .iter_mut()
            .find(|mob| mob.id == 70)
            .unwrap()
            .invulnerable = 0;
        game.mobs.iter_mut().find(|mob| mob.id == 70).unwrap().age += 1;
        assert!(hit(game, &arrow(None), 100.0));
        let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
        game.mob_died(&dead);
    });
}

fn player(game: &mut Game<'_>) {
    game.player = Some(horizon_client::motion::PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    crate::game::native_entity_refs::tests::bind_local_player(game, 11, 123);
}
#[test]
#[ignore = "daily"]
fn death_weapon_riptide_iii_uses_spin_trident() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let mut trident = ItemStack::new("minecraft:trident", 1);
        trident.enchant("minecraft:riptide", 3);
        fixture(Some(trident.clone()), |game| {
            player(game);
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
            let motion = game.player.as_mut().unwrap();
            motion.auto_spin_attack_ticks = 20;
            motion.auto_spin_attack_item = Some(trident);
            let mut dead = victim(game);
            dead.last_damage = Some((Damage::player([0.0; 3]), 0));
            dead.health = 0.0;
            dead.ai.last_hurt_by_player = 100;
            game.remember_mob_player_credit(dead.id);
            game.mob_died(&dead);
        });
    });
}
#[test]
#[ignore = "daily"]
fn death_weapon_harming_potion_does_not_read_thrower_sword() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        player(game);
        game.inventory[game.selected] = Some(ItemStack::new("minecraft:diamond_sword", 1));
        let mut target = victim(game);
        target.health = 1.0;
        let at = target.position;
        game.mobs.push(target);
        let mut potion = ItemStack::new("minecraft:splash_potion", 1);
        potion.data_mut().potion = Some("minecraft:harming".into());
        let owner = game.potion_owner(Some(Who::Player));
        game.splash(
            &potion,
            at,
            Some(at),
            None,
            3,
            &owner,
            true,
            (None, Vec3::ZERO),
        );
        let dead = game.mobs.iter().find(|mob| mob.id == 70).unwrap().clone();
        assert_eq!(dead.health, 0.0);
        assert_eq!(dead.last_direct, Some("minecraft:splash_potion"));
        assert_eq!(
            game.mob_damage_source(&dead),
            ("minecraft:indirect_magic", 11)
        );
        game.mob_died(&dead);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_different_killing_source_discards_projectile() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        game.mobs.push(victim(game));
        assert!(hit(
            game,
            &arrow(Some(ItemStack::new("minecraft:bow", 1))),
            1.0
        ));
        assert!(game.mobs[0].accepted_hit.is_some());
        let mut dead = game.mobs[0].clone();
        dead.last_damage = Some((Damage::of(DamageKind::Fall), i64::from(dead.age) + 1));
        dead.health = 0.0;
        game.mob_died(&dead);
        assert!(game.mobs[0].accepted_hit.is_none());
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_without_subscriber_does_not_capture() {
    if isolated() {
        return;
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                1.0
            ));
            assert!(game.mobs[0].accepted_hit.is_none());
        });
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_removal_clears_capture_and_vehicle_transfer_keeps_its_owner() {
    if isolated() {
        return;
    }
    fixture(None, |game| {
        for transfer in [false, true] {
            game.mobs.clear();
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:bow", 1))),
                1.0
            ));
            assert!(game.mobs[0].accepted_hit.is_some());
            if transfer {
                let mut state = game.take_transferred_entity_state(70);
                game.detach_entity(70);
                assert!(
                    game.mobs[0].accepted_hit.is_none(),
                    "the transfer token took ownership"
                );
                game.mobs[0].last_damage = None;
                state.restore_mob(&mut game.mobs[0]);
                assert!(
                    game.mobs[0]
                        .accepted_hit
                        .as_ref()
                        .unwrap()
                        .belongs_to(game.mobs[0].last_damage)
                );
                game.adopt_transferred_entity_state(70, state);
            } else {
                game.forget_entity(70);
            }
            assert_eq!(game.mobs[0].accepted_hit.is_some(), transfer);
        }
        let mut dead = victim(game);
        dead.health = 0.0;
        game.mob_died(&dead);
    });
}

#[test]
#[ignore = "daily"]
fn death_weapon_same_tick_stronger_arrow_replaces_weapon() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        if isolated() {
            return;
        }
        let expected = ItemStack::new("minecraft:bow", 1);
        fixture(Some(expected.clone()), |game| {
            game.mobs.push(victim(game));
            assert!(hit(
                game,
                &arrow(Some(ItemStack::new("minecraft:crossbow", 1))),
                1.0
            ));
            let first = game.mobs.iter().find(|m| m.id == 70).unwrap().last_damage;
            assert!(hit(game, &arrow(Some(expected.clone())), 2.0));
            let mut dead = game.mobs.iter().find(|m| m.id == 70).unwrap().clone();
            assert_eq!(
                dead.last_damage, first,
                "equal source values still commit a new hit"
            );
            assert_eq!(
                dead.accepted_hit.as_ref().unwrap().weapon,
                Some(expected.clone())
            );
            dead.health = 0.0;
            game.mob_died(&dead);
        });
    });
}
