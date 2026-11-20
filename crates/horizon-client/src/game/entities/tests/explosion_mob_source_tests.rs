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
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(VICTIM, |g| {
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
                            after
                                .into_iter()
                                .zip(before)
                                .all(|(after, before)| after < before)
                        );
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
