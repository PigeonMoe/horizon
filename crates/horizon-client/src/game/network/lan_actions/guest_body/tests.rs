use super::*;
use horizon_client::{
    assets::Assets,
    lan::{Joined, PlayerSender, guest::SavedBody},
};
use horizon_core::random::RandomSource;
use horizon_protocol::packets::configuration::ClientInformation;
use std::sync::{Arc, atomic::AtomicI32, mpsc};

#[path = "tests/fall_reset.rs"]
mod fall_reset;
#[path = "tests/fire_contact.rs"]
mod fire_contact;
#[path = "tests/liquid_contact.rs"]
mod liquid_contact;
#[path = "tests/fall_contact.rs"]
mod fall_contact;
#[path = "tests/sprint_attempt.rs"]
mod sprint_attempt;
#[path = "tests/starvation.rs"]
mod starvation;

#[test]
#[ignore = "daily"]
fn accepted_fall_dispatches_one_reaction_at_the_actual_landing() {
    use horizon_client::{items::ItemStack, mobs::Mob};
    use mod_api::{Flow, Hooks, ModEntry, ServerHost};
    use std::sync::Mutex;

    const REACTION: u128 = 77;

    struct Probe(Arc<Mutex<Vec<[f64; 3]>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn living_incoming_damage(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            if event.source.damage_type == "minecraft:fall" {
                event.container.set_new_damage(8.0);
            } else if event.source.damage_type == "minecraft:generic" {
                assert_eq!((event.source.direct, event.source.causing), (0, 0));
            }
            Flow::Continue
        }

        fn entity_damage_target_pre(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::EntityDamageTargetPreEvent<'_>,
        ) -> Flow {
            if let Some(actor) = event.player.filter(|actor| actor.uuid == 12) {
                assert_eq!(actor.position, [3.0, 64.0, 5.0]);
                if event.position[0] < 4.0 {
                    return Flow::Cancel;
                }
            }
            Flow::Continue
        }

        fn player_fall_reaction(
            &self,
            _: &dyn mod_api::LoopContext,
            event: &mut mod_api::PlayerFallReactionEvent<'_>,
        ) -> Flow {
            assert_eq!(event.actor.0, 12);
            assert_eq!(
                event.boots.item(),
                mod_api::__private::item_key("minecraft:leather_boots").unwrap()
            );
            assert_eq!(event.damage, 8.0);
            *event.effect = (REACTION, None);
            Flow::Cancel
        }

        fn player_fall_reaction_post(
            &self,
            host: &mut dyn ServerHost,
            event: &mut mod_api::PlayerFallReactionPostEvent,
        ) {
            let player = event.player.entity();
            assert_eq!(event.reaction, REACTION);
            assert_eq!(event.player.uuid(), 12);
            assert_eq!(event.bounds[0][1], event.origin[1]);
            assert!(player.set_velocity(host, [0.0, 0.7, 0.0]));
            self.0.lock().unwrap().push(event.origin);
            for id in [13, 31, 32] {
                let target = host.entity(id).unwrap();
                assert!(target.set_velocity(host, [1.4, 0.0, 0.0]));
                let allowed = target.allows_player_damage(host, event.player);
                assert_eq!(allowed, Some(id == 32));
                if allowed == Some(true) {
                    assert_eq!(
                        target.hurt(host, "minecraft:generic", event.damage / 2.0, None, None),
                        Some(true)
                    );
                }
            }
        }
    }

    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "fall-reaction",
            Box::new(Probe(calls.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                assert!(
                    game.ui
                        .rules
                        .set("pvp", horizon_client_core::gamerules::Value::Bool(false))
                );
                let state = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set(x, 63, z, state.0);
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let pig = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                for (id, x) in [(31, 3.5), (32, 5.0)] {
                    game.mobs
                        .push(Mob::new(id, pig.clone(), Vec3::new(x, 64.0, 5.0), 0.0));
                }
                let host = game.lan.as_mut().unwrap();
                host.hub
                    .update_guest(13, |guest| {
                        assert!(initialize(game.motion, guest));
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    })
                    .unwrap();
                host.hub
                    .update_guest(12, |guest| {
                        assert!(initialize(game.motion, guest));
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.armor[3] = Some(ItemStack::new("minecraft:leather_boots", 1));
                        guest.moves = vec![
                            ([0.0, -9.0, 0.0], false),
                            ([0.0, -1.0, 0.0], true),
                            ([0.2, 0.0, 0.0], true),
                        ];
                        guest.position = [3.2, 64.0, 5.0];
                        guest.on_ground = true;
                    })
                    .unwrap();
                game.guest_server_tick(12);
                assert_eq!(&*calls.lock().unwrap(), &[[3.0, 64.0, 5.0]]);
                assert_eq!(game.mobs[0].health, 10.0);
                assert_eq!(game.mobs[1].health, 6.0);
                assert_eq!(game.mobs[0].velocity.x, 1.4);
                let host = game.lan.as_ref().unwrap();
                assert_eq!(host.hub.guest(12).unwrap().health, 20.0);
                assert_eq!(host.hub.guest(13).unwrap().health, 20.0);
                assert_eq!(
                    host.hub
                        .guest(13)
                        .unwrap()
                        .motion
                        .as_ref()
                        .unwrap()
                        .velocity
                        .x,
                    1.4
                );
                game.guest_server_tick(12);
                assert_eq!(calls.lock().unwrap().len(), 1);

                let actor = game.player(12).unwrap().entity();
                assert_eq!(actor.damage_cooldown_time(game), Some(0));
                assert_eq!(
                    actor.hurt(game, "minecraft:generic", 12.0, None, None),
                    Some(true)
                );
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| {
                        guest.moves = vec![([0.0, -9.0, 0.0], false), ([0.0, -1.0, 0.0], true)];
                    })
                    .unwrap();
                game.guest_server_tick(12);
                assert_eq!(
                    calls.lock().unwrap().len(),
                    1,
                    "cooldown rejected the second fall"
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guest(12).unwrap().health,
                    8.0
                );
            });
        });
    });
}

fn guest(entity_id: i32) -> Guest {
    let (sender, _) = mpsc::channel();
    Guest::new(
        Joined {
            conn: entity_id as u32,
            uuid: entity_id as u128,
            name: "GuestBodyTest".into(),
            properties: Vec::new(),
            info: ClientInformation::new(2),
            brand: "test".into(),
            sender: PlayerSender::new(sender, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        },
        entity_id,
        [0.5, 64.0, 0.5],
        2,
    )
}

#[test]
#[ignore = "daily"]
fn co_located_guests_own_distinct_streams_and_restore_the_saved_body() {
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();
    let mut owner = PlayerMotion::new(&resources, Vec3::new(0.5, 64.0, 0.5)).unwrap();
    let mut first = guest(2);
    let mut second = guest(3);
    let saved = SavedBody {
        fire_ticks: 43,
        air: 173,
        fall_distance: 4.5,
        velocity: [0.25, -0.5, 0.75],
        food_exhaustion: 1.25,
        food_tick_timer: 17,
        ticks_frozen: 23,
        invulnerable: true,
        invulnerable_time: 7,
    };
    first.loaded_body = Some(saved);
    initialize(&resources, &mut first);
    initialize(&resources, &mut second);
    assert!(first.loaded_body.is_none());
    assert_eq!(SavedBody::of(first.motion.as_ref().unwrap()), saved);
    let mut first_random = first.motion.as_ref().unwrap().random.clone();
    let mut second_random = second.motion.as_ref().unwrap().random.clone();
    let first_trace: Vec<_> = (0..4).map(|_| first_random.next_int()).collect();
    let second_trace: Vec<_> = (0..4).map(|_| second_random.next_int()).collect();
    let owner_trace: Vec<_> = (0..4).map(|_| owner.random.next_int()).collect();
    assert_ne!(first_trace, second_trace);
    assert_ne!(first_trace, owner_trace);
    assert_ne!(second_trace, owner_trace);
    // Advancing one guest does not borrow or advance the other body.
    first.motion.as_mut().unwrap().random.next_int();
    let unchanged: Vec<_> = (0..4)
        .map(|_| second.motion.as_mut().unwrap().random.next_int())
        .collect();
    assert_eq!(unchanged, second_trace);
}

#[test]
#[ignore = "daily"]
fn accepted_hit_moves_through_teleport_restore_but_not_respawn() {
    use horizon_client::health::{AcceptedHit, Attack, HurtSource};
    use horizon_client::items::ItemStack;
    let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
    let assets = Assets::load(&root).unwrap();
    let resources = MotionResources::load(&root, &assets).unwrap();
    let source = HurtSource {
        damage_type: "minecraft:arrow",
        causing: 41,
        direct: 73,
        position: Some([-17.25, 64.5, 32.75]),
    };
    let mut weapon = ItemStack::new("minecraft:bow", 1);
    weapon.damage = 19;
    weapon.enchant("minecraft:power", 4);
    let attack = Attack {
        attacker: 41,
        projectile: Some(73),
        projectile_weapon: Some(weapon.clone()),
        ..Default::default()
    };
    let hit = Arc::new(AcceptedHit::capture(source, &attack, None));
    for restore_all in [true, false] {
        let mut player = guest(2);
        assert!(initialize(&resources, &mut player));
        player.motion.as_mut().unwrap().health.accepted_hit = Some(hit.clone());
        let previous = player.new_body(restore_all).unwrap();
        if restore_all {
            assert!(
                previous.health.accepted_hit.is_none(),
                "the record moved, not copied"
            );
            assert!(Arc::ptr_eq(player.carried_hit.as_ref().unwrap(), &hit));
            assert!(
                player.new_body(true).is_none(),
                "a second teleport keeps the pending owner"
            );
        } else {
            assert!(
                player.carried_hit.is_none(),
                "a new living identity inherits no accepted blow"
            );
        }
        assert!(initialize(&resources, &mut player));
        assert!(
            player.carried_hit.is_none(),
            "initialization consumes the pending owner once"
        );
        let restored = player.motion.as_ref().unwrap().health.accepted_hit.as_ref();
        if restore_all {
            let restored = restored.unwrap();
            assert!(Arc::ptr_eq(restored, &hit));
            assert_eq!(restored.source(), source);
            assert_eq!(restored.weapon, Some(weapon.clone()));
            player.new_body(true);
            assert!(player.carried_hit.is_some());
            player.new_body(false);
            assert!(
                player.carried_hit.is_none(),
                "respawn clears even an unmaterialized teleport body"
            );
        } else {
            assert!(restored.is_none());
        }
    }
}
