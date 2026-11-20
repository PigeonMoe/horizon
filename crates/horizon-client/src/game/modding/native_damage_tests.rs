//! `living_damage_pre` and `living_damage_post` heard by a trait mod, through the game's own
//! hooks: the damage container each hit fills, the hit's level host, and the mob and dragon
//! hits that fire them.
use super::dragon::{Dragon, SPECIAL};
use super::entity::damage_level_of;
use super::native_entity_refs::tests::{bind_local_player, with_game};
use horizon_client::command::args::GameMode;
use horizon_client::mobs::{Damage, DamageKind, Mob};
use horizon_client::motion::PlayerMotion;
use horizon_core::{geometry::Vec3, random::RandomSource};
use mod_api::{
    self, Hooks, HurtSource, LevelRandomHost, LivingDamagePostEvent, LivingDamagePreEvent,
    LivingIncomingDamageEvent, LivingShieldBlockEvent, Reduction,
};
use std::sync::{Arc, Mutex};

/// What the probe heard: Pre's entity, original damage, new damage and armor's reduction, and
/// Post's health damage, then its entity, health damage, absorption reduction and blocked damage.
#[derive(Debug, Default, PartialEq)]
struct Heard {
    pre: Vec<(i32, f32, f32, f32)>,
    post: Vec<(i32, f32)>,
    post_values: Vec<(i32, f32, f32, f32)>,
    sources: Vec<HitSource>,
    side_effects: Vec<(i32, bool)>,
}

#[derive(Debug, PartialEq)]
struct HitSource {
    phase: &'static str,
    entity: i32,
    causing: i32,
    direct: i32,
}

impl Heard {
    fn source(&mut self, phase: &'static str, entity: i32, source: &HurtSource<'_>) {
        self.sources.push(HitSource {
            phase,
            entity,
            causing: source.causing,
            direct: source.direct,
        });
    }
}

/// The test mod: 225001's incoming hit becomes 4, Pre sets 225003's and 225008's damage to 2,
/// and draws once from the level's random for 225010.
struct Probe(Arc<Mutex<Heard>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .lock()
            .unwrap()
            .source("incoming", event.entity, event.source);
        if event.entity == 225001 {
            event.container.set_new_damage(4.0);
        }
        mod_api::Flow::Continue
    }
    fn living_shield_block(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingShieldBlockEvent<'_>,
    ) {
        if event.entity == 227002 {
            event.block.blocked = true;
            event.block.blocked_damage = event.damage;
        }
    }
    fn living_damage_pre(
        &self,
        level: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePreEvent<'_>,
    ) {
        let container = &mut *event.container;
        if matches!(event.entity, 225003 | 225008) {
            container.set_new_damage(2.0);
        }
        if event.entity == 225010 {
            level.with_random(|random| random.next_int()).unwrap();
        }
        let heard = (
            event.entity,
            container.original(),
            container.new_damage(),
            container.reduction(Reduction::Armor),
        );
        let mut log = self.0.lock().unwrap();
        log.pre.push(heard);
        log.source("pre", event.entity, event.source);
    }
    fn living_damage_post(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePostEvent<'_>,
    ) {
        let heard = (event.entity, event.container.health_damage());
        let mut log = self.0.lock().unwrap();
        log.post.push(heard);
        log.post_values.push((
            event.entity,
            event.container.health_damage(),
            event.container.reduction(Reduction::Absorption),
            event.container.blocked(),
        ));
        log.source("post", event.entity, event.source);
        log.side_effects
            .push((event.entity, event.container.should_cause_side_effects()));
    }
}

/// Runs `test` in a game with the probe installed, and returns what it heard.
fn heard(test: impl FnOnce(&mut super::Game<'_>)) -> Heard {
    let log = Arc::new(Mutex::new(Heard::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "probe",
        Box::new(Probe(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || with_game(test));
    std::mem::take(&mut *log.lock().unwrap())
}

fn mob(game: &super::Game<'_>, id: u32, kind: &str) -> Mob {
    let kind = game.res.mob_types().get(kind).unwrap().clone();
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

#[test]
#[ignore = "daily"]
fn damage_admission_checks_the_player_responsible_for_the_actual_hit() {
    struct Listener(Arc<Mutex<Vec<Option<u128>>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.target_kind, "minecraft:pig");
            assert!(event.alive_kind);
            self.0.lock().unwrap().push(event.player.map(|p| p.uuid));
            if event.player.is_some() {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "damage-admission",
        Box::new(Listener(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let id = 225012;
            survival_player(game, id);
            let attack = game.attack_of(id as i32);
            let mut pig = mob(game, 225013, "minecraft:pig");
            assert!(!pig.damage(
                damage_level_of!(game, &attack),
                3.0,
                Damage::mob(id, [2.0, 0.0, 2.0]),
            ));
            assert_eq!(pig.health, 10.0);
            assert!(pig.damage(
                damage_level_of!(game, &attack),
                3.0,
                Damage::mob(id + 2, [2.0, 0.0, 2.0]),
            ));
            assert_eq!(pig.health, 7.0);
        })
    });
    assert_eq!(*log.lock().unwrap(), [Some(225012), None]);
}

#[test]
#[ignore = "daily"]
fn a_player_damage_permission_query_does_not_damage_its_target() {
    struct ProtectPig;

    #[mod_api::__subscriptions]
    impl Hooks for ProtectPig {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.target_kind, "minecraft:pig");
            assert!(event.alive_kind);
            assert_eq!(event.player.map(|p| p.uuid), Some(225023));
            if event.position[0] < 3.0 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let mods = vec![mod_api::ModEntry::new("protect-pig", Box::new(ProtectPig))].into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.build_registries();
                survival_player(game, 225023);
                let player = game.player(225023).unwrap();
                for (id, x, permitted) in [(225024, 2.0, false), (225025, 4.0, true)] {
                    let mut pig = mob(game, id, "minecraft:pig");
                    pig.position.x = x;
                    game.mobs.push(pig);
                    let target = game.entity(id).unwrap();
                    assert_eq!(target.allows_player_damage(game, player), Some(permitted));
                    assert_eq!(game.mobs.last().unwrap().health, 10.0);
                }
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn armor_stand_sweeps_and_sourced_damage_check_the_living_target() {
    use super::commands::Target;
    use super::mace::Spared;
    use horizon_client::entities::ArmorStand;

    struct ProtectStand;

    #[mod_api::__subscriptions]
    impl Hooks for ProtectStand {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.target_kind, "minecraft:armor_stand");
            assert!(event.alive_kind);
            assert_eq!(event.player.map(|player| player.uuid), Some(225020));
            if event.position[0] < 2.0 {
                mod_api::Flow::Cancel
            } else {
                mod_api::Flow::Continue
            }
        }
    }

    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let mods = vec![mod_api::ModEntry::new(
            "protect-stand",
            Box::new(ProtectStand),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                survival_player(game, 225020);
                game.world.game_time = 100;
                for (id, x) in [(225021, 1.5), (225022, 2.5)] {
                    game.armor_stands
                        .push(ArmorStand::new(id, Vec3::new(x, 0.0, 0.0), 0.0));
                }
                let target = game.armor_stands[0].bounds();
                let protected_last_hit = game.armor_stands[0].last_hit;
                game.sweep_damage(target, Spared::Nothing, 1.0);
                assert_eq!(game.armor_stands[0].last_hit, protected_last_hit);
                assert_eq!(game.armor_stands[1].last_hit, 100);

                for (id, allowed) in [(225021, false), (225022, true)] {
                    assert_eq!(
                        game.hurt_target(
                            Target::Stand(id),
                            2.0,
                            "minecraft:arrow",
                            Some(Target::Player),
                            None,
                            None,
                        ),
                        allowed
                    );
                }
                assert_eq!(game.armor_stands.len(), 1);
                assert_eq!(game.armor_stands[0].id, 225021);
                assert!(!game.hurt_target(
                    Target::Stand(225021),
                    2.0,
                    "minecraft:generic_kill",
                    Some(Target::Player),
                    None,
                    None,
                ));
                assert_eq!(game.armor_stands.len(), 1);
                // A source-free hit does not raise the by-entity protection event.
                assert!(game.hurt_target(
                    Target::Stand(225021),
                    2.0,
                    "minecraft:arrow",
                    None,
                    None,
                    None,
                ));
                assert!(game.armor_stands.is_empty());
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn mob_drops_record_the_players_credit_on_an_environmental_death() {
    struct Listener(Arc<Mutex<Vec<Option<u128>>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Listener {
        fn living_drops(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::LivingDropsEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.source.causing, 0);
            assert_eq!(event.source.damage_type, "minecraft:fall");
            self.0.lock().unwrap().push(event.credited_player);
            mod_api::Flow::Continue
        }
    }

    let log = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "mob-drop-credit",
        Box::new(Listener(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let id = 225014;
            survival_player(game, id);
            game.load_server_loot();
            let mut pig = mob(game, 225015, "minecraft:pig");
            pig.last_damage = Some((Damage::of(DamageKind::Fall), game.world.game_time));
            pig.health = 0.0;
            pig.ai.last_hurt_by_player = 20;
            game.last_hurt_by_player
                .insert(pig.id, super::combat::PlayerCredit(u128::from(id)));
            game.mobs.push(pig.clone());
            game.mob_died(&pig);
        });
    });
    assert_eq!(*log.lock().unwrap(), [Some(225014)]);
}

#[test]
#[ignore = "daily"]
fn the_original_damage_is_the_hit_before_the_incoming_hook() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225001, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game),
            10.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 6.0);
    });
    assert_eq!(heard.pre, [(225001, 10.0, 4.0, 0.0)]);
    assert_eq!(heard.post, [(225001, 4.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_sets_the_health_lost_and_the_last_hurt_the_cooldown_compares() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225003, "minecraft:pig");
        assert!(pig.damage(damage_level_of!(game), 6.0, Damage::of(DamageKind::Generic)));
        assert_eq!(pig.health, 8.0);
        // The inflicted damage reaches `lastHurt`.
        assert_eq!(pig.last_hurt, 2.0);
    });
    assert_eq!(heard.pre, [(225003, 6.0, 2.0, 0.0)]);
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_fires_for_a_hit_its_reductions_took_to_zero() {
    let heard = heard(|game| {
        let mut witch = mob(game, 225008, "minecraft:witch");
        // Witch.getDamageAfterMagicAbsorb: none from its own doing.
        let own = Damage::mob(225008, [2.0, 0.0, 2.0]);
        assert!(witch.damage(damage_level_of!(game), 6.0, own));
        assert_eq!(witch.health, 24.0);
    });
    assert_eq!(heard.pre, [(225008, 6.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn pre_draws_from_the_level_random_of_the_hit() {
    heard(|game| {
        let mut expected = game.world.random.clone();
        expected.next_int();
        let mut pig = mob(game, 225010, "minecraft:pig");
        assert!(pig.damage(damage_level_of!(game), 1.0, Damage::of(DamageKind::Generic)));
        assert_eq!(game.world.random.next_int(), expected.next_int());
    });
}

#[test]
#[ignore = "daily"]
fn a_hit_on_the_dragon_fires_pre_and_post() {
    let heard = heard(|game| {
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225009;
        game.dragon.replace(dragon);
        let _ = game.hit_special(SPECIAL, 6.0, true, DamageKind::Generic, None);
        assert_eq!(game.dragon.as_ref().unwrap().health, 194.0);
    });
    assert_eq!(heard.pre, [(225009, 6.0, 6.0, 0.0)]);
    assert_eq!(heard.post, [(225009, 6.0)]);
}

#[test]
#[ignore = "daily"]
fn mob_and_dragon_post_records_fresh_hit_decision() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225014, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(damage_level_of!(game), 2.0, source));
        assert!(pig.damage(damage_level_of!(game), 4.0, source));
        assert!(!pig.damage(damage_level_of!(game), 3.0, source));
        assert_eq!(pig.health, 6.0);
        let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
        dragon.id = 225015;
        game.dragon.replace(dragon);
        for amount in [2.0, 4.0, 3.0] {
            let _ = game.hit_special(SPECIAL, amount, true, DamageKind::Generic, None);
        }
        assert_eq!(game.dragon.as_ref().unwrap().health, 196.0);
    });
    assert_eq!(
        heard.post,
        [(225014, 2.0), (225014, 2.0), (225015, 2.0), (225015, 2.0)]
    );
    assert_eq!(
        heard.side_effects,
        [
            (225014, true),
            (225014, false),
            (225015, true),
            (225015, false)
        ]
    );
}

fn survival_player(game: &mut super::Game<'_>, id: u32) {
    game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
    bind_local_player(game, id, u128::from(id));
    game.set_game_mode(GameMode::Survival);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_that_absorption_took_whole() {
    let heard = heard(|game| {
        survival_player(game, 227001);
        // Absorption is cut to `max_absorption`, 0 without a modifier.
        let player = game.player.as_mut().unwrap();
        player
            .set_modifier(
                "minecraft:max_absorption",
                "test:absorption",
                10.0,
                "add_value",
                true,
            )
            .unwrap();
        player.set_health(2.0, true).unwrap();
        assert!(game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false));
        let health = &game.player.as_ref().unwrap().health;
        assert_eq!((health.health(), health.absorption()), (20.0, 0.0));
    });
    assert_eq!(heard.post_values, [(227001, 0.0, 2.0, 0.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_for_a_player_hit_a_shield_blocked_whole() {
    let heard = heard(|game| {
        survival_player(game, 227002);
        game.hurt_player_from(2.0, None, "minecraft:generic", Some(None), false);
        assert_eq!(game.player.as_ref().unwrap().health.health(), 20.0);
    });
    assert_eq!(heard.post_values, [(227002, 0.0, 0.0, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn a_hit_inside_the_cooldown_no_stronger_than_the_last_fires_no_post() {
    let heard = heard(|game| {
        // Pre sets 225003's damage to 2, which `lastHurt` keeps.
        let mut pig = mob(game, 225003, "minecraft:pig");
        let source = Damage::of(DamageKind::Generic);
        assert!(pig.damage(damage_level_of!(game), 4.0, source));
        // Inside the cooldown and no stronger than the last hit: refused, no Post.
        assert!(!pig.damage(damage_level_of!(game), 2.0, source));
    });
    assert_eq!(heard.post, [(225003, 2.0)]);
}

#[test]
#[ignore = "daily"]
fn post_fires_once_for_a_killing_blow_on_a_mob() {
    let heard = heard(|game| {
        let mut pig = mob(game, 225016, "minecraft:pig");
        assert!(pig.damage(
            damage_level_of!(game),
            100.0,
            Damage::of(DamageKind::Generic)
        ));
        assert_eq!(pig.health, 0.0);
    });
    // `living_death` is settled by the game's next tick, after the hurt and its Post.
    assert_eq!(heard.post, [(225016, 100.0)]);
}

/// What a killing blow tells a mod, in order: `("post", entity)` and `("death", entity)`.
#[derive(Default)]
struct DeathOrder {
    heard: Mutex<Vec<(&'static str, i64)>>,
    cancel: std::sync::atomic::AtomicBool,
}

struct DeathProbe(Arc<DeathOrder>);

#[mod_api::__subscriptions]
impl Hooks for DeathProbe {
    fn living_damage_post(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingDamagePostEvent<'_>,
    ) {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("post", entity));
    }
    fn living_death(
        &self,
        _: &mut dyn mod_api::ServerHost,
        event: &mut mod_api::LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        let entity = i64::from(event.entity);
        self.0.heard.lock().unwrap().push(("death", entity));
        if self.0.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

/// A pig killed by a 100-damage hit through the game, then the creature tick that settles its
/// death and one more: what the probe heard, and the pig's health after.
fn killing_blow(cancel: bool) -> (Vec<(&'static str, i64)>, f32) {
    let probe = Arc::new(DeathOrder::default());
    probe
        .cancel
        .store(cancel, std::sync::atomic::Ordering::Relaxed);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "death-order",
        Box::new(DeathProbe(probe.clone())),
    )]
    .into();
    let mut health = f32::NAN;
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            let pig = mob(game, 225017, "minecraft:pig");
            game.mobs.push(pig);
            let victim = game.entity(225017).unwrap();
            assert_eq!(
                victim.hurt(game, "minecraft:magic", 100.0, None, None),
                Some(true)
            );
            game.tick_creature_entities().unwrap();
            game.tick_creature_entities().unwrap();
            health = game.mobs.iter().find(|m| m.id == 225017).unwrap().health;
        });
    });
    let heard = std::mem::take(&mut *probe.heard.lock().unwrap());
    (heard, health)
}

/// Horizon's order, not NeoForge's (whose Post follows the death processing): a killing blow's
/// Post fires once, then the death hooks, as the maintainers ruled on arismoko/horizon#3963.
#[test]
#[ignore = "daily"]
fn a_killing_blows_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(false),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

/// A death a mod cancels: the pig remains cancelled at 0 health with `death_canceled`,
/// without a second Post or death on the next tick.
#[test]
#[ignore = "daily"]
fn a_cancelled_deaths_post_fires_once_before_the_death_hooks() {
    assert_eq!(
        killing_blow(true),
        (vec![("post", 225017), ("death", 225017)], 0.0)
    );
}

#[test]
#[ignore = "daily"]
fn the_container_names_what_armor_took() {
    let heard = heard(|game| {
        let mut zombie = mob(game, 225011, "minecraft:zombie");
        // A mob's attack: armor takes its share, where a generic hit bypasses it.
        let attack = Damage::mob(225012, [2.0, 0.0, 2.0]);
        assert!(zombie.damage(damage_level_of!(game), 10.0, attack));
    });
    // A zombie's two armor points against 10 damage: `CombatRules.getDamageAfterAbsorb`
    // clamps 2 - 10 / 2 up to a fifth of the armor, 0.4, and keeps 1 - 0.4 / 25 of the hit.
    let after = 10.0 * (1.0 - 0.4_f32 / 25.0);
    assert_eq!(heard.pre, [(225011, 10.0, after, 10.0 - after)]);
}

/// A level built the normal way lends the game's hooks to its hits: a path that built it
/// without them would run no mod's damage hook.
#[test]
#[ignore = "daily"]
fn a_game_built_level_has_every_damage_hook() {
    with_game(|game| {
        let hooks = &game.damage_hooks;
        assert!(hooks.incoming.is_some() && hooks.pre.is_some() && hooks.post.is_some());
        assert!(hooks.knockback.is_some() && hooks.totem.is_some() && hooks.shield.is_some());
    });
}

/// The real projectile hit routes (not hand-built HurtSources) preserve the direct
/// entity through incoming, pre and post, independently of whether there is a shooter.
#[test]
#[ignore = "daily"]
fn arrow_hits_name_the_projectile_on_players_mobs_and_the_dragon() {
    use horizon_client::command::args::GameMode;
    use horizon_client::health::NO_ATTACK;
    use horizon_client::mobs::Who;
    use horizon_client::motion::PlayerMotion;
    use horizon_client::projectiles::{Hit, Projectile, ProjectileKind};

    for (target, owner, entity, causing) in [
        (Who::Player, Some(Who::Mob(42)), 7, 42),
        (Who::Player, None, 7, 0),
        (Who::Mob(43), Some(Who::Player), 43, 7),
        (Who::Mob(43), None, 43, 0),
        (Who::Mob(SPECIAL), Some(Who::Player), 44, 7),
    ] {
        let heard = heard(|game| {
            let mut player = PlayerMotion::new(game.motion, Vec3::ZERO).unwrap();
            player.entity_id = 7;
            game.player = Some(player);
            game.set_game_mode(GameMode::Survival);
            game.mobs.push(mob(game, 42, "minecraft:skeleton"));
            game.mobs.push(mob(game, 43, "minecraft:pig"));
            let mut dragon = Dragon::new(Vec3::ZERO, 0.0, 0);
            // Hover is a sitting phase and correctly shrugs off arrows.
            dragon.set_native_phase(mod_api::DragonPhase::HoldingPattern);
            dragon.id = 44;
            game.dragon.replace(dragon);
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(0.0, 1.0, 0.5),
                owner,
            );
            arrow.id = 912;
            arrow.velocity = Vec3::new(0.0, 0.0, -2.0);
            game.projectile_hit(
                &arrow,
                Hit::Entity {
                    who: target,
                    at: Vec3::ZERO,
                },
            )
            .unwrap();
            assert_eq!(game.current_attack(), NO_ATTACK, "projectile scope leaked");
            match target {
                Who::Player => assert!(game.player.as_ref().unwrap().health.health() < 20.0),
                Who::Mob(SPECIAL) => assert!(game.dragon.as_ref().unwrap().health < 200.0),
                Who::Mob(_) => assert!(game.mobs[1].health < 10.0),
            }
        });
        let expected = ["incoming", "pre", "post"].map(|phase| HitSource {
            phase,
            entity,
            causing,
            direct: 912,
        });
        assert_eq!(
            heard.sources, expected,
            "target {target:?}, owner {owner:?}"
        );
    }
}

#[test]
#[ignore = "daily"]
fn an_admitted_arrow_contacts_a_crystal_before_it_explodes() {
    use super::dragon::{CRYSTAL_IDS, Crystal};
    use horizon_client::{
        mobs::Who,
        projectiles::{Hit, Projectile, ProjectileKind},
    };
    use mod_api::{ArrowContactPostEvent, ExplosionStartEvent, Flow, ServerHost};

    #[derive(Debug, PartialEq)]
    enum Reached {
        Contact(u32, [f64; 3]),
        Explosion,
    }

    struct CrystalProbe(Arc<Mutex<Vec<Reached>>>);

    #[mod_api::__subscriptions]
    impl Hooks for CrystalProbe {
        fn arrow_contact_post(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ArrowContactPostEvent<'_>,
        ) {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.owner, 0);
            assert_eq!(event.contact, "damage");
            self.0
                .lock()
                .unwrap()
                .push(Reached::Contact(event.projectile, event.at));
        }

        fn explosion_start(&self, _: &mut dyn ServerHost, _: &mut ExplosionStartEvent<'_>) -> Flow {
            self.0.lock().unwrap().push(Reached::Explosion);
            Flow::Cancel
        }
    }

    let observed = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "crystal_arrow_probe",
        Box::new(CrystalProbe(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let at = Vec3::new(8.5, 70.0, 8.5);
            game.crystals.push(Crystal {
                attachment_lifetime: Default::default(),
                join: Default::default(),
                id: 42,
                position: at,
                show_bottom: false,
                time: 0,
                beam: None,
                invulnerable: true,
                fall: (false, 0.0),
                motion: Vec3::ZERO,
                rotation: [0.0; 2],
            });
            let mut arrow = Projectile::new(
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                Vec3::new(7.5, 70.5, 8.5),
                None,
            );
            arrow.id = 81;
            arrow.velocity = Vec3::new(2.0, 0.0, 0.0);
            let hit = Hit::Entity {
                who: Who::Mob(CRYSTAL_IDS),
                at: arrow.position,
            };

            game.projectile_hit(&arrow, hit).unwrap();
            assert_eq!(game.crystals.len(), 1);
            assert!(observed.lock().unwrap().is_empty());

            game.crystals[0].invulnerable = false;
            arrow.id = 82;
            game.projectile_hit(&arrow, hit).unwrap();
            assert!(game.crystals.is_empty());
            assert_eq!(
                observed.lock().unwrap().as_slice(),
                &[Reached::Contact(82, [7.5, 70.5, 8.5]), Reached::Explosion]
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn an_arrow_contacts_a_dragon_only_after_admitted_living_damage() {
    use horizon_client::{
        mobs::Who,
        projectiles::{Hit, Projectile, ProjectileKind},
    };
    use mod_api::{ArrowContactPostEvent, EntityDamageTargetPreEvent, Flow, ServerHost};

    const PLAYER: u32 = 225026;
    const DRAGON: i32 = 225027;
    const ZEROED_ARROW: u32 = 86;
    const CONTACT: Vec3 = Vec3::new(7.5, 71.0, 8.5);

    #[derive(Debug, PartialEq)]
    enum Reached {
        Damage(i32, f32, bool),
        Contact(u32),
    }

    struct DragonProbe(Arc<Mutex<Vec<Reached>>>);

    #[mod_api::__subscriptions]
    impl Hooks for DragonProbe {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.target_kind, "minecraft:ender_dragon");
            assert_eq!(
                event.player.map(|player| player.uuid),
                Some(u128::from(PLAYER))
            );
            if event.position[0] < 10.0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn living_damage_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingDamagePreEvent<'_>,
        ) {
            if event.source.direct == ZEROED_ARROW as i32 {
                event.container.set_new_damage(0.0);
            }
        }

        fn living_damage_post(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut LivingDamagePostEvent<'_>,
        ) {
            assert_eq!(event.entity, DRAGON);
            self.0.lock().unwrap().push(Reached::Damage(
                event.source.direct,
                event.container.health_damage(),
                event.container.should_cause_side_effects(),
            ));
        }

        fn arrow_contact_post(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ArrowContactPostEvent<'_>,
        ) {
            assert_eq!(event.owner, PLAYER);
            assert_eq!(event.at, CONTACT.components());
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.contact, "damage");
            self.0
                .lock()
                .unwrap()
                .push(Reached::Contact(event.projectile));
        }
    }

    super::native_entity_refs::tests::run_on_guest_stack(|| {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![mod_api::ModEntry::new(
            "dragon-arrow-probe",
            Box::new(DragonProbe(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                survival_player(game, PLAYER);
                let dragon = |x, flying| {
                    let mut dragon = Dragon::new(Vec3::new(x, 70.0, 8.5), 0.0, 0);
                    dragon.id = DRAGON as u32;
                    if flying {
                        dragon.set_native_phase(mod_api::DragonPhase::HoldingPattern);
                    }
                    dragon
                };
                let impact = |game: &mut super::Game<'_>, id, speed, owner, kind| {
                    let mut projectile = Projectile::new(kind, CONTACT, owner);
                    projectile.id = id;
                    projectile.velocity = Vec3::new(speed, 0.0, 0.0);
                    let hit = Hit::Entity {
                        who: Who::Mob(SPECIAL),
                        at: CONTACT,
                    };
                    game.projectile_hit(&projectile, hit).unwrap();
                    game.dragon.as_ref().unwrap().health
                };
                let arrow = || ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                };
                let player = Some(Who::Player);

                game.dragon.replace(dragon(8.5, true));
                assert_eq!(impact(game, 80, 2.0, player, arrow()), 200.0);
                game.dragon.replace(dragon(12.5, false));
                assert_eq!(impact(game, 81, 2.0, player, arrow()), 200.0);
                game.dragon.replace(dragon(12.5, true));
                assert_eq!(impact(game, 82, 2.0, None, arrow()), 200.0);
                assert!(observed.lock().unwrap().is_empty());

                assert_eq!(impact(game, 83, 2.0, player, arrow()), 196.0);
                assert_eq!(impact(game, 84, 1.0, player, arrow()), 196.0);
                assert_eq!(impact(game, 85, 3.0, player, arrow()), 194.0);

                game.dragon.replace(dragon(12.5, true));
                assert_eq!(impact(game, ZEROED_ARROW, 2.0, player, arrow()), 200.0);
                game.dragon.replace(dragon(12.5, true));
                assert_eq!(
                    impact(game, 87, 2.0, player, ProjectileKind::Trident),
                    192.0
                );
            });
        });
        assert_eq!(
            observed.lock().unwrap().as_slice(),
            &[
                Reached::Damage(83, 4.0, true),
                Reached::Contact(83),
                Reached::Damage(85, 2.0, false),
                Reached::Contact(85),
                Reached::Damage(ZEROED_ARROW as i32, 0.0, true),
                Reached::Contact(ZEROED_ARROW),
                Reached::Damage(87, 8.0, true),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn snapshot_crosses_the_real_damage_hook_with_reductions() {
    let log = heard(|game| {
        let mut container = horizon_world::living_damage::DamageContainer::new(10.0);
        container.record_reduction(horizon_world::living_damage::Reduction::Armor, 3.5);
        container.enter_pre(6.5);
        damage_level_of!(game).damage_pre(
            horizon_client::health::Hurting::Player(
                225003,
                HurtSource {
                    damage_type: "minecraft:generic",
                    causing: 0,
                    direct: 0,
                    position: None,
                },
            ),
            &mut container,
        );
        assert_eq!(container.capture_inflicted(), 2.0);
        assert_eq!(
            container.reduction(horizon_world::living_damage::Reduction::Armor),
            3.5
        );
    });
    assert_eq!(log.pre, [(225003, 10.0, 2.0, 3.5)]);
}

#[test]
#[ignore = "daily"]
fn unsubscribed_damage_does_not_convert() {
    mod_platform::testing::with_mods(Vec::<mod_api::ModEntry>::new().into(), || {
        with_game(|game| {
            super::api_values::damage::CONVERSIONS.with(|count| count.set(0));
            let mut container = horizon_world::living_damage::DamageContainer::new(10.0);
            damage_level_of!(game).damage_pre(
                horizon_client::health::Hurting::Player(
                    225003,
                    HurtSource {
                        damage_type: "minecraft:generic",
                        causing: 0,
                        direct: 0,
                        position: None,
                    },
                ),
                &mut container,
            );
            super::api_values::damage::CONVERSIONS.with(|count| assert_eq!(count.get(), 0));
        })
    });
}

#[test]
#[ignore = "daily"]
fn a_flying_arrow_hurts_only_an_admitted_non_marker_armor_stand() {
    use horizon_client::{
        entities::ArmorStand,
        mobs::Who,
        projectiles::{Projectile, ProjectileKind},
    };
    use mod_api::{ArrowHitArmorStandPostEvent, Flow, ServerHost};

    const PLAYER: u32 = 225040;
    const DENIED: u32 = 225041;
    const MARKER: u32 = 225042;
    const ALLOWED: u32 = 225043;
    struct Proof(Arc<Mutex<Vec<(u32, u32, [f64; 3], f32)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Proof {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.target_kind, "minecraft:armor_stand");
            assert_eq!(
                event.player.map(|player| player.uuid),
                Some(u128::from(PLAYER))
            );
            assert!(event.alive_kind);
            if event.position[2] < 2.0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn arrow_hit_armor_stand_post(
            &self,
            _: &mut dyn ServerHost,
            event: &mut ArrowHitArmorStandPostEvent<'_>,
        ) {
            assert_eq!(event.dimension, "minecraft:overworld");
            self.0
                .lock()
                .unwrap()
                .push((event.stand, event.owner, event.position, event.damage));
        }
    }

    let heard = Arc::new(Mutex::new(Vec::new()));
    let during_hit = heard.clone();
    crate::game::native_entity_refs::tests::run_on_guest_stack(move || {
        let mods = vec![mod_api::ModEntry::new(
            "arrow-stand",
            Box::new(Proof(during_hit)),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                survival_player(game, PLAYER);
                game.options.dedicated = Some(Vec::new());
                game.build_registries();
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                for (id, x, z) in [(DENIED, 4., 0.5), (MARKER, 2., 4.5), (ALLOWED, 4., 4.5)] {
                    let mut stand = ArmorStand::new(id, Vec3::new(x, 64., z), 0.);
                    stand.marker = id == MARKER;
                    game.armor_stands.push(stand);
                }
                for z in [0.5, 4.5] {
                    let mut arrow = Projectile::new(
                        ProjectileKind::Arrow {
                            base_damage: 2.,
                            critical: false,
                            effect: None,
                        },
                        Vec3::new(1., 64.75, z),
                        Some(Who::Player),
                    );
                    arrow.velocity = Vec3::new(4., 0., 0.);
                    game.spawn_mob_projectile(arrow);
                }
                for _ in 0..3 {
                    game.tick_projectiles_now().unwrap();
                }
                assert!(
                    game.armor_stands
                        .iter()
                        .any(|stand| stand.id == DENIED && stand.health == 20.)
                );
                assert!(game.armor_stands.iter().any(|stand| stand.id == MARKER));
                assert!(!game.armor_stands.iter().any(|stand| stand.id == ALLOWED));
            })
        });
    });
    let observed = heard.lock().unwrap();
    let [(stand, owner, position, raw)] = &observed[..] else {
        panic!("one admitted stand hit")
    };
    assert_eq!(
        (*stand, *owner, *position),
        (ALLOWED, PLAYER, [4., 64., 4.5])
    );
    assert!(raw.is_finite() && *raw > 0.);
}

#[test]
#[ignore = "daily"]
fn a_player_damage_permission_query_reports_the_actual_cross_dimension_shooter() {
    struct Protect;
    #[mod_api::__subscriptions]
    impl Hooks for Protect {
        fn entity_damage_target_pre(
            &self,
            _: &mut dyn LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> mod_api::Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            let player = event.player.expect("the online original shooter");
            assert_eq!(
                (player.uuid, player.dimension.as_str()),
                (12, "minecraft:the_nether")
            );
            assert_eq!(player.position, [7., 69., 3.]);
            mod_api::Flow::Cancel
        }
    }
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let mods = vec![mod_api::ModEntry::new(
            "cross-world-splash",
            Box::new(Protect),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.dimension = "minecraft:the_nether".into();
                        guest.position = [7., 69., 3.];
                    })
                    .unwrap();
                let shooter = game.player(12).unwrap();
                let id = 225044;
                let pig = mob(game, id, "minecraft:pig");
                game.mobs.push(pig);
                let target = game.entity(id).unwrap();
                assert_eq!(target.allows_player_damage(game, shooter), Some(false));
                assert_eq!(game.mobs.last().unwrap().health, 10.);
            });
        });
    });
}
