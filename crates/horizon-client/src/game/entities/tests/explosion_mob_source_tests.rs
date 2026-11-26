//! `Explosion.getIndirectSourceEntity` for a blast a mob caused: a mob-owned
//! primed TNT spares vehicles without `mobGriefing`, and a mob's own blast
//! hands its kind and UUID to the TNT it primes.
use super::*;
use crate::game::boats::Boat;
use crate::game::game_tests::tests::server_game;
use horizon_client::gamerules::Value;
use horizon_client::mobs::{Mob, MobEvent, Species};
use horizon_core::nbt::Tag;

/// Blasts a boat 7.7 blocks away with `owner` as the indirect source, and
/// returns what the boat took: its damage and its push.
fn blast_boat(
    game: &mut Game<'_>,
    owner: Option<(u32, Text)>,
    player: bool,
    mob_griefing: bool,
) -> (f32, Vec3) {
    game.boats.clear();
    game.boats
        .push(Boat::new(1, "oak".into(), Vec3::new(16.2, 64.0, 8.5), 0.0));
    assert!(game.ui.rules.set("mob_griefing", Value::Bool(mob_griefing)));
    game.combat.blast_owner = owner;
    game.combat.blast_owner_player = player;
    game.server_explode([8.5, 64.0, 8.5], 4.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.boats.len(), 1, "the boat survives the blast");
    let boat = &game.boats[0];
    (boat.damage, boat.velocity)
}

#[test]
#[ignore = "daily"]
fn a_mob_owned_blast_spares_vehicles_only_without_mob_griefing() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
    game.mobs
        .push(Mob::new(7, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
    let pig = || (7, Text::translate("entity.minecraft.pig", &[]));
    let steve = (1, Text::literal("Steve"));
    // VehicleEntity.ignoreExplosion: the indirect source is a mob and
    // mobGriefing is off, so the boat is neither hurt nor pushed.
    let (damage, velocity) = blast_boat(&mut game, Some(pig()), false, false);
    assert_eq!(damage, 0.0, "a mob's blast spares the boat");
    assert_eq!(velocity, Vec3::ZERO, "and does not push it");
    // The same blast with mobGriefing on still hits it.
    let (damage, velocity) = blast_boat(&mut game, Some(pig()), false, true);
    assert!(damage > 0.0, "mobGriefing on lets the boat be hurt");
    assert_ne!(velocity, Vec3::ZERO, "and pushed");
    // A player-owned TNT is not a mob: mobGriefing off does not spare it.
    let (damage, velocity) = blast_boat(&mut game, Some(steve), true, false);
    assert!(damage > 0.0, "a player's blast still hurts the boat");
    assert_ne!(velocity, Vec3::ZERO, "and pushes it");
    // PrimedTnt.getOwner no longer resolves a removed mob.
    game.mobs[0].removed = true;
    let (damage, _) = blast_boat(&mut game, Some(pig()), false, false);
    assert!(damage > 0.0, "a removed owner no longer spares the boat");
}

#[test]
#[ignore = "daily"]
fn a_deflected_ghast_fireballs_blast_protects_targets_against_the_actual_shooter() {
    use crate::game::{
        native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
    };
    use horizon_client::projectiles::{Projectile, ProjectileKind};
    use mod_api::{EntityDamageTargetPreEvent, Flow, Hooks, LevelRandomHost, ModEntry};
    use std::sync::{Arc, Mutex};

    const SHOOTER: u32 = 13;
    const VICTIM: u32 = 12;
    const GHAST: u32 = 70;
    const PIG: u32 = 71;
    const FIREBALL: u32 = 81;

    #[derive(Clone, Default)]
    struct Claim(Arc<Mutex<Vec<(bool, Option<u128>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Claim {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            let victim = event.target_kind == "minecraft:player" && event.position[2] < 4.0;
            if !victim && event.target_kind != "minecraft:pig" {
                return Flow::Continue;
            }
            assert_eq!(event.dimension, "minecraft:overworld");
            assert!(event.alive_kind);
            let responsible = event.player.map(|player| player.uuid);
            if let Some(player) = event.player {
                assert_eq!(player.name, "guest13");
                assert_eq!(player.position, [3.0, 64.0, 5.0]);
            }
            self.0.lock().unwrap().push((victim, responsible));
            if responsible == Some(u128::from(SHOOTER)) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    run_on_guest_stack(|| {
        for reflected in [false, true] {
            let claim = Claim::default();
            let mods: mod_api::Mods =
                vec![ModEntry::new("fireball_claim", Box::new(claim.clone()))].into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.load_ticking_neighbourhood(0, 0);
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(5, 5));
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    // Placed guests still have join immunity until their clients load.
                    for conn in [SHOOTER, VICTIM] {
                        hub.update_guest(conn, |g| {
                            g.loaded = true;
                            g.loading = false;
                            g.load_timeout = 0;
                        })
                        .unwrap();
                    }
                    hub.update_guest(VICTIM, |g| {
                        g.position = [7.0, 64.0, 3.5];
                        g.previous_position = g.position;
                    })
                    .unwrap();
                    let stone = game
                        .pack
                        .assets
                        .registry
                        .default_state("minecraft:stone")
                        .unwrap();
                    game.world.set(8, 65, 5, stone).unwrap();
                    for (id, kind, at) in [
                        (GHAST, "minecraft:ghast", Vec3::new(13.0, 73.0, 13.0)),
                        (PIG, "minecraft:pig", Vec3::new(7.0, 65.0, 6.8)),
                    ] {
                        let kind = game.res.mob_types().get(kind).unwrap().clone();
                        game.mobs.push(Mob::new(id, kind, at, 0.0));
                    }
                    let pig_health = |game: &Game<'_>| {
                        game.mobs.iter().find(|mob| mob.id == PIG).unwrap().health
                    };
                    let player_health = |game: &Game<'_>| {
                        game.lan.as_ref().unwrap().hub.guest(VICTIM).unwrap().health
                    };
                    let before = [player_health(game), pig_health(game)];
                    let mut shot = Projectile::fireball(
                        ProjectileKind::LargeFireball { power: 2 },
                        Vec3::new(5.0, 65.62, 5.0),
                        Some(Who::Mob(GHAST)),
                        [1.0, 0.0, 0.0],
                    );
                    shot.id = FIREBALL;
                    shot.owner_uuid = Some(game.uuid_for(GHAST));
                    game.creatures.projectiles.push(shot);
                    if reflected {
                        let hub = &mut game.lan.as_mut().unwrap().hub;
                        hub.update_guest(SHOOTER, |guest| {
                            guest.yaw = -90.0;
                            guest.pitch = 0.0;
                        })
                        .unwrap();
                        let gi = hub.guests.iter().position(|g| g.conn == SHOOTER).unwrap();
                        assert!(game.guest_attack(SHOOTER, gi, FIREBALL as i32).unwrap());
                        let fireball = &game.creatures.projectiles[0];
                        assert_eq!(
                            (fireball.owner, fireball.guest),
                            (Some(Who::Player), Some(SHOOTER))
                        );
                    }
                    // Both victims are beside the line of flight; normal projectile
                    // ticks must fly to the wall before their splash protection runs.
                    for _ in 0..20 {
                        game.tick_projectiles().unwrap();
                        if game.creatures.projectiles.is_empty() {
                            break;
                        }
                    }
                    assert!(
                        game.creatures.projectiles.is_empty(),
                        "the fireball never hit the wall"
                    );
                    let expected = reflected.then_some(u128::from(SHOOTER));
                    let mut checks = claim.0.lock().unwrap();
                    checks.sort_unstable();
                    assert_eq!(*checks, [(false, expected), (true, expected)]);
                    let after = [player_health(game), pig_health(game)];
                    if reflected {
                        assert_eq!(after, before);
                    } else {
                        assert!(
                            after[0] < before[0],
                            "unreflected blast: guest {before:?} -> {after:?}"
                        );
                        assert!(
                            after[1] < before[1],
                            "unreflected blast: pig {before:?} -> {after:?}"
                        );
                    }
                });
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn an_end_crystals_breaker_is_not_its_boat_or_cart_damage_attacker() {
    use crate::game::{
        minecarts::{CartKind, Minecart},
        native_entity_refs::tests::run_on_guest_stack,
        native_guest_turn_tests::with_guests,
    };
    use mod_api::{Flow, Hooks, Host, ModEntry, VehicleDamagePreEvent};
    use std::sync::{Arc, Mutex};

    const OWNER: u32 = 13;
    const PROTECTED_BOAT: u32 = 90;
    const PROTECTED_CART: u32 = 91;
    const OPEN_BOAT: u32 = 92;
    const OPEN_CART: u32 = 93;

    #[derive(Default)]
    struct Claim(Mutex<Vec<(u32, Option<u128>)>>);

    struct Protection(Arc<Claim>);

    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn vehicle_damage_pre(&self, _: &dyn Host, event: &mut VehicleDamagePreEvent<'_>) -> Flow {
            let attacker = event.player.map(|player| player.uuid);
            self.0.0.lock().unwrap().push((event.vehicle, attacker));
            let protected = matches!(event.vehicle, PROTECTED_BOAT | PROTECTED_CART);
            if protected && attacker != Some(u128::from(OWNER)) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    run_on_guest_stack(|| {
        for crystal in [true, false] {
            let claim = Arc::new(Claim::default());
            let mods = vec![ModEntry::new(
                "vehicle_claim",
                Box::new(Protection(claim.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.load_ticking_neighbourhood(0, 0);
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(6, 6));
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    for conn in [12, OWNER] {
                        hub.update_guest(conn, |guest| {
                            guest.loaded = true;
                            guest.loading = false;
                            guest.load_timeout = 0;
                        })
                        .unwrap();
                    }
                    hub.update_guest(OWNER, |guest| {
                        guest.position = [3.0, 100.0, 6.0];
                        guest.previous_position = guest.position;
                    })
                    .unwrap();
                    for (id, at) in [
                        (PROTECTED_BOAT, Vec3::new(7.5, 100.0, 4.5)),
                        (OPEN_BOAT, Vec3::new(4.5, 100.0, 8.5)),
                    ] {
                        game.boats.push(Boat::new(id, "oak".into(), at, 0.0));
                    }
                    for (id, at) in [
                        (PROTECTED_CART, Vec3::new(4.5, 100.0, 4.5)),
                        (OPEN_CART, Vec3::new(7.5, 100.0, 8.5)),
                    ] {
                        game.carts.push(Minecart::new(id, CartKind::Rideable, at));
                    }
                    crate::game::sculk::TEST_POSTED_EVENTS
                        .with(|events| *events.borrow_mut() = Some(Vec::new()));
                    if crystal {
                        game.as_console(|game| {
                            assert!(game.run_command(
                                r#"summon minecraft:end_crystal 6.5 100 6.5 {Tags:["vehicle_crystal"]}"#
                            ));
                            assert!(game.run_command(
                                "damage @e[tag=vehicle_crystal,limit=1] 1 minecraft:player_attack by guest13"
                            ));
                        });
                        assert!(
                            game.crystals.is_empty(),
                            "the ordinary damage route broke the crystal"
                        );
                    } else {
                        game.add_primed_tnt(
                            BlockPos::new(6, 100, 6),
                            0.0,
                            1,
                            Some(Who::Player),
                            Some(OWNER),
                        );
                        game.tick_one_tnt(0, &[]).unwrap();
                        assert!(game.tnts.is_empty(), "the player-primed TNT detonated");
                    }
                    let events = crate::game::sculk::TEST_POSTED_EVENTS
                        .with(|events| events.borrow_mut().take().unwrap());
                    let vehicle_credits: Vec<_> = events
                        .into_iter()
                        .filter(|(event, [x, y, z], _)| {
                            *event == "minecraft:entity_damage"
                                && [4.5, 7.5].contains(x)
                                && *y == 100.0
                                && [4.5, 8.5].contains(z)
                        })
                        .collect();
                    assert_eq!(vehicle_credits.len(), if crystal { 2 } else { 4 });
                    assert!(
                        vehicle_credits
                            .into_iter()
                            .all(|(_, _, source)| source == Some(Who::Player)),
                        "an admitted vehicle hit retains the breaker's combat credit"
                    );
                    let expected_attacker = (!crystal).then_some(u128::from(OWNER));
                    let mut calls = claim.0.lock().unwrap();
                    calls.sort_unstable();
                    assert_eq!(
                        *calls,
                        [
                            (PROTECTED_BOAT, expected_attacker),
                            (PROTECTED_CART, expected_attacker),
                            (OPEN_BOAT, expected_attacker),
                            (OPEN_CART, expected_attacker),
                        ]
                    );
                    assert!(game.boats.iter().all(|boat| boat.id != OPEN_BOAT));
                    assert!(game.carts.iter().all(|cart| cart.id != OPEN_CART));
                    if crystal {
                        assert_eq!(game.boats[0].id, PROTECTED_BOAT);
                        assert_eq!(game.boats[0].damage, 0.0);
                        assert_eq!(game.carts[0].id, PROTECTED_CART);
                        assert_eq!(game.carts[0].damage, 0.0);
                        // Permission attribution must not change ordinary living
                        // damage or turn a crystal into a PvP-only explosion.
                        let owner = game.lan.as_ref().unwrap().hub.guest(OWNER).unwrap();
                        assert!(owner.health < 20.0);
                    } else {
                        assert!(game.boats.is_empty());
                        assert!(game.carts.is_empty());
                    }
                });
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_creeper_blast_primes_tnt_with_the_creeper_as_its_owner() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    let creeper = 77;
    let kind = game
        .res
        .mob_types()
        .get("minecraft:creeper")
        .unwrap()
        .clone();
    game.mobs
        .push(Mob::new(creeper, kind, Vec3::new(8.5, 64.0, 8.5), 0.0));
    let uuid = game.uuid_for(creeper);
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    game.world.set(9, 64, 8, tnt).unwrap();
    let name = Text::translate("entity.minecraft.creeper", &[]);
    game.handle_mob_events(vec![
        MobEvent::ExplosionSource {
            mob: creeper,
            charged: false,
            name,
            in_water: false,
            kind: "minecraft:creeper".into(),
        },
        MobEvent::Explode([8.5, 64.0, 8.5], 3.0, Species::Creeper, false),
    ])
    .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block");
    assert_eq!(
        game.tnts[0].owner,
        Some(Who::Mob(creeper)),
        "the chain TNT remembers the creeper"
    );
    // PrimedTnt.addAdditionalSaveData writes the owner's UUID.
    let saved = game.tnt_nbt(0);
    assert_eq!(saved.get("owner"), Some(&Tag::IntArray(uuid.to_vec())));
}

#[test]
#[ignore = "daily"]
fn a_projectiles_owner_primes_tnt_and_a_gone_owner_does_not() {
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.build_registries();
    let ghast = 91;
    let kind = game.res.mob_types().get("minecraft:ghast").unwrap().clone();
    game.mobs
        .push(Mob::new(ghast, kind, Vec3::new(40.0, 64.0, 40.0), 0.0));
    let uuid = game.uuid_for(ghast);
    let tnt = game
        .pack
        .assets
        .registry
        .default_state("minecraft:tnt")
        .unwrap();
    // `LargeFireball.onHit` and `WitherSkull.onHit` name the projectile's owner
    // as the blast's indirect source (`Explosion.getIndirectSourceEntity`).
    game.combat.explosion_owner = Some(ghast);
    game.world.set(9, 64, 8, tnt).unwrap();
    game.server_explode([8.5, 64.0, 8.5], 3.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block");
    assert_eq!(
        game.tnts[0].owner,
        Some(Who::Mob(ghast)),
        "the chain TNT remembers the fireball's owner"
    );
    assert_eq!(
        game.tnt_nbt(0).get("owner"),
        Some(&Tag::IntArray(uuid.to_vec()))
    );
    // `Projectile.getOwner` is an `EntityReference`: a removed owner does not
    // resolve, so the TNT it primes is left without one.
    game.tnts.clear();
    game.mobs[0].removed = true;
    game.world.set(9, 64, 8, tnt).unwrap();
    game.server_explode([8.5, 64.0, 8.5], 3.0, BlockInteraction::Destroy, false)
        .unwrap();
    assert_eq!(game.tnts.len(), 1, "the blast primed the TNT block again");
    assert_eq!(game.tnts[0].owner, None, "a gone owner primes no one");
    game.combat.explosion_owner = None;
}

/// A blast given an explicit cause of none (a crystal or cart source with no
/// entity) credits no one with what it breaks, even while a primed TNT's owner
/// is known: the explicit source wins, `DamageSource.getEntity` is null.
#[test]
#[ignore = "daily"]
fn an_explicit_cause_of_none_credits_no_one() {
    use crate::game::commands::Target;
    let mut resources = None;
    let mut game = server_game(&mut resources);
    game.combat.blast_owner = Some((7, Text::literal("Steve")));
    game.combat.blast_owner_player = true;
    assert_eq!(
        game.explosion_kill_credit(),
        Some(Target::Player),
        "the TNT owner"
    );
    game.combat.explicit_explosion_cause = Some(None);
    assert_eq!(
        game.explosion_kill_credit(),
        None,
        "an explicit cause of none"
    );
}

#[test]
#[ignore = "daily"]
fn a_tnt_minecart_keeps_its_igniters_living_credit_and_uses_the_cart_for_protection() {
    use crate::game::{
        minecarts::{CartKind, Minecart},
        native_entity_refs::tests::run_on_guest_stack,
        native_guest_turn_tests::with_guests,
    };
    use mod_api::{
        EntityDamageTargetPreEvent, Flow, HangingBreakEvent, Hooks, Host, LevelRandomHost,
        LivingDeathEvent, LivingIncomingDamageEvent, ModEntry, ServerHost, VehicleDamagePreEvent,
    };
    use std::sync::{Arc, Mutex};

    const SHOOTER: u32 = 13;
    const GUEST_VICTIM: u32 = 12;
    const TNT_CART: u32 = 87;
    const PIG: u32 = 88;
    const PROTECTED_BOAT: u32 = 90;
    const ORDINARY_CART: u32 = 91;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Ignition {
        GuestArrow,
        GuestArrowOtherDimension,
        PlayerPrimedTnt,
        UnownedArrow,
        UnownedFuse,
    }

    #[derive(Default)]
    struct Observations {
        vehicle: Mutex<Vec<(u32, Option<u128>)>>,
        policy: Mutex<Vec<(String, Option<u128>)>>,
        hanging: Mutex<Vec<(String, Option<u128>)>>,
        living: Mutex<Vec<(i32, String, i32, i32)>>,
        death: Mutex<Vec<(u32, String, i32, i32)>>,
    }

    struct Probe(Arc<Observations>);

    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn vehicle_damage_pre(&self, _: &dyn Host, event: &mut VehicleDamagePreEvent<'_>) -> Flow {
            let actor = event.player.map(|player| player.uuid);
            if matches!(event.vehicle, PROTECTED_BOAT | ORDINARY_CART) {
                self.0.vehicle.lock().unwrap().push((event.vehicle, actor));
            }
            if event.vehicle == PROTECTED_BOAT && actor != Some(u128::from(SHOOTER)) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            if matches!(
                event.target_kind,
                "minecraft:player"
                    | "minecraft:pig"
                    | "minecraft:item_frame"
                    | "minecraft:painting"
            ) {
                self.0.policy.lock().unwrap().push((
                    event.target_kind.to_owned(),
                    event.player.map(|player| player.uuid),
                ));
            }
            Flow::Continue
        }

        fn hanging_break(&self, _: &mut dyn ServerHost, event: &mut HangingBreakEvent<'_>) -> Flow {
            self.0
                .hanging
                .lock()
                .unwrap()
                .push((event.kind.to_owned(), event.player));
            if event.player == Some(u128::from(SHOOTER)) {
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }

        fn living_incoming_damage(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            if [GUEST_VICTIM as i32, PIG as i32].contains(&event.entity) {
                let source = event.source;
                self.0.living.lock().unwrap().push((
                    event.entity,
                    source.damage_type.to_owned(),
                    source.causing,
                    source.direct,
                ));
            }
            Flow::Continue
        }

        fn living_death(&self, _: &mut dyn ServerHost, event: &mut LivingDeathEvent<'_>) -> Flow {
            if event.entity == PIG {
                let source = event.source;
                self.0.death.lock().unwrap().push((
                    event.entity,
                    source.damage_type.to_owned(),
                    source.causing,
                    source.direct,
                ));
            }
            Flow::Continue
        }
    }

    crate::entry::install_client_hooks();
    run_on_guest_stack(|| {
        for ignition in [
            Ignition::GuestArrow,
            Ignition::GuestArrowOtherDimension,
            Ignition::PlayerPrimedTnt,
            Ignition::UnownedArrow,
            Ignition::UnownedFuse,
        ] {
            let seen = Arc::new(Observations::default());
            let mods = vec![ModEntry::new(
                "cart_ignition_probe",
                Box::new(Probe(seen.clone())),
            )]
            .into();
            mod_platform::testing::with_mods(mods, || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.load_ticking_neighbourhood(0, 0);
                    game.update_chunk_tickets();
                    assert!(game.entity_ticking_area()(6, 6));
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    for (id, position) in [
                        (SHOOTER, [32.0, 200.0, 32.0]),
                        (GUEST_VICTIM, [30.0, 200.0, 30.0]),
                    ] {
                        hub.update_guest(id, |guest| {
                            guest.loaded = true;
                            guest.loading = false;
                            guest.load_timeout = 0;
                            guest.position = position;
                            guest.previous_position = position;
                        })
                        .unwrap();
                    }
                    let mut cart =
                        Minecart::new(TNT_CART, CartKind::Tnt, Vec3::new(6.5, 200.0, 6.5));
                    cart.no_gravity = true;
                    if ignition == Ignition::UnownedFuse {
                        cart.fuse = 1;
                    }
                    game.carts.push(cart);
                    if ignition == Ignition::PlayerPrimedTnt {
                        game.add_primed_tnt(
                            BlockPos::new(6, 200, 6),
                            0.0,
                            1,
                            Some(Who::Player),
                            Some(SHOOTER),
                        );
                        game.tick_one_tnt(0, &[]).unwrap();
                        assert!(game.tnts.is_empty(), "the ordinary TNT detonated first");
                        assert_eq!(game.carts[0].id, TNT_CART);
                        assert!(
                            game.carts[0].fuse >= 0,
                            "the owned blast ignited a timed fuse"
                        );
                        game.carts.update(0, |cart| cart.velocity = Vec3::ZERO);
                    }
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(GUEST_VICTIM, |guest| {
                            guest.position = [8.5, 200.0, 6.5];
                            guest.previous_position = guest.position;
                        })
                        .unwrap();
                    game.boats.push(Boat::new(
                        PROTECTED_BOAT,
                        "oak".into(),
                        Vec3::new(6.5, 200.0, 4.5),
                        0.0,
                    ));
                    game.carts.push(Minecart::new(
                        ORDINARY_CART,
                        CartKind::Rideable,
                        Vec3::new(6.5, 200.0, 8.5),
                    ));
                    let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                    game.mobs
                        .push(Mob::new(PIG, pig, Vec3::new(8.5, 200.0, 8.5), 0.0));
                    game.as_console(|game| {
                        for command in [
                            "setblock 6 200 3 minecraft:obsidian",
                            "setblock 9 200 5 minecraft:obsidian",
                            "summon minecraft:item_frame 5.5 200.5 3.5 {Facing:4b}",
                            "summon minecraft:painting 8.5 200.5 5.5 {Facing:4b,variant:\"minecraft:kebab\"}",
                        ] {
                            assert!(game.run_command(command), "{command}");
                        }
                    });

                    let mut arrow_cause = 0;
                    if matches!(ignition, Ignition::PlayerPrimedTnt | Ignition::UnownedFuse) {
                        for _ in 0..45 {
                            game.tick_carts();
                            if game.carts.iter().all(|cart| cart.id != TNT_CART) {
                                break;
                            }
                        }
                    } else {
                        let owned = matches!(
                            ignition,
                            Ignition::GuestArrow | Ignition::GuestArrowOtherDimension
                        );
                        let owner = if owned { ",Owner:[I;0,0,0,13]" } else { "" };
                        game.as_console(|game| {
                            assert!(game.run_command(&format!(
                                "summon minecraft:arrow 4.5 200.4 6.5 {{NoGravity:1b,Motion:[2d,0d,0d],Fire:80s{owner}}}"
                            )));
                        });
                        let arrow = &game.creatures.projectiles[0];
                        if !owned {
                            arrow_cause = arrow.id as i32;
                        }
                        assert_eq!(arrow.owner, owned.then_some(Who::Player));
                        assert_eq!(arrow.guest, owned.then_some(SHOOTER));
                        assert!(arrow.arrow.as_ref().unwrap().on_fire());
                        if ignition == Ignition::GuestArrowOtherDimension {
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(SHOOTER, |guest| {
                                    guest.dimension = "minecraft:the_nether".into();
                                })
                                .unwrap();
                        }
                        for _ in 0..4 {
                            game.tick_projectiles().unwrap();
                            if game.carts.iter().all(|cart| cart.id != TNT_CART) {
                                break;
                            }
                        }
                    }

                    assert!(
                        game.carts.iter().all(|cart| cart.id != TNT_CART),
                        "{ignition:?}: cart never detonated"
                    );
                    assert!(game.carts.iter().all(|cart| cart.id != ORDINARY_CART));
                    assert_eq!(game.boats[0].id, PROTECTED_BOAT);
                    assert_eq!(game.boats[0].damage, 0.0);
                    assert_eq!(
                        game.item_frames.len(),
                        1,
                        "hanging policy protected the frame"
                    );
                    assert_eq!(
                        game.paintings.len(),
                        1,
                        "hanging policy protected the painting"
                    );

                    let mut vehicle = seen.vehicle.lock().unwrap();
                    vehicle.sort_unstable();
                    assert_eq!(*vehicle, [(PROTECTED_BOAT, None), (ORDINARY_CART, None)]);
                    let mut policy = seen.policy.lock().unwrap();
                    policy.sort_unstable();
                    assert_eq!(
                        policy
                            .iter()
                            .map(|(kind, actor)| (kind.as_str(), *actor))
                            .collect::<Vec<_>>(),
                        [
                            ("minecraft:item_frame", None),
                            ("minecraft:painting", None),
                            ("minecraft:pig", None),
                            ("minecraft:player", None)
                        ]
                    );
                    let mut hanging = seen.hanging.lock().unwrap();
                    hanging.sort_unstable();
                    assert_eq!(
                        hanging
                            .iter()
                            .map(|(kind, actor)| (kind.as_str(), *actor))
                            .collect::<Vec<_>>(),
                        [("minecraft:item_frame", None), ("minecraft:painting", None)]
                    );

                    let credited = matches!(
                        ignition,
                        Ignition::GuestArrow
                            | Ignition::GuestArrowOtherDimension
                            | Ignition::PlayerPrimedTnt
                    );
                    let causing = if credited {
                        SHOOTER as i32
                    } else {
                        arrow_cause
                    };
                    let kind = if causing == 0 {
                        "minecraft:explosion"
                    } else {
                        "minecraft:player_explosion"
                    };
                    let direct = TNT_CART as i32;

                    let mut living = seen.living.lock().unwrap();
                    living.sort_unstable();
                    eprintln!(
                        "{ignition:?}: vehicle={vehicle:?} policy={policy:?} hanging={hanging:?} living={living:?}"
                    );
                    assert_eq!(
                        living
                            .iter()
                            .map(|(id, kind, cause, direct)| (*id, kind.as_str(), *cause, *direct))
                            .collect::<Vec<_>>(),
                        [
                            (GUEST_VICTIM as i32, kind, causing, direct),
                            (PIG as i32, kind, causing, direct)
                        ]
                    );
                    assert_eq!(
                        game.last_hurt_by_player.get(&PIG).map(|credit| credit.0),
                        credited.then_some(u128::from(SHOOTER))
                    );
                    assert!(
                        game.lan
                            .as_ref()
                            .unwrap()
                            .hub
                            .guest(GUEST_VICTIM)
                            .unwrap()
                            .health
                            < 20.0
                    );
                    drop((vehicle, policy, hanging, living));
                    game.tick_creature_entities().unwrap();
                    let deaths = seen.death.lock().unwrap();
                    assert_eq!(
                        deaths
                            .iter()
                            .map(|(id, kind, cause, direct)| (*id, kind.as_str(), *cause, *direct))
                            .collect::<Vec<_>>(),
                        [(PIG, kind, causing, direct)]
                    );
                });
            });
        }
    });
}
