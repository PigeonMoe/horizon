//! The food tick's starvation hit pauses while the real guest is installed.
use super::initialize;
use horizon_client::chunk::Chunk;
use mod_api::{Flow, Hooks, Host, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Case {
    Allowed,
    Denied,
    Cooldown,
    Early,
    NoStarvation,
}

struct Probe(Arc<Mutex<(Case, Vec<&'static str>)>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        if event.source.damage_type == "minecraft:starve" {
            assert_eq!((event.source.direct, event.source.causing), (0, 0));
            self.0.lock().unwrap().1.push("incoming");
        }
        Flow::Continue
    }

    fn player_environment_damage_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerEnvironmentDamagePreEvent<'_>,
    ) -> Flow {
        if event.damage_type != "minecraft:starve" {
            return Flow::Continue;
        }
        let player = event.player;
        assert_eq!(player.uuid(), 12);
        assert_eq!(host.players().acting_player(), Some(player));
        assert_eq!((player.food(host).unwrap().level, event.damage), (0, 1.0));
        assert_eq!(
            host.living(player.entity()).unwrap().health(host),
            Some(20.0)
        );
        assert!(
            player
                .inventory_slots(host)
                .unwrap()
                .iter()
                .any(|(slot, stack)| *slot == 7 && stack.item().id(host) == "minecraft:potion")
        );
        let mut observed = self.0.lock().unwrap();
        observed.1.push("environment");
        if observed.0 == Case::Denied {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn starvation_runs_the_guest_environment_hook_only_for_an_admitted_hit() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let observed = Arc::new(Mutex::new((Case::Allowed, Vec::new())));
        let mods = vec![ModEntry::new(
            "starvation",
            Box::new(Probe(observed.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                let ground = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut chunk = Chunk::empty(0, 0);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set(x, 63, z, ground.0);
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let player = game.player(12).unwrap();
                let juice = (&*game as &dyn Host)
                    .item_from_snbt(r#"{id:"minecraft:potion",count:1}"#)
                    .unwrap();
                player.set_inventory_slot(game, 7, Some(juice)).unwrap();
                for (case, food, timer, expected_health, order) in [
                    (Case::Allowed, 0, 79, 19.0, &["incoming", "environment"][..]),
                    (Case::Denied, 0, 79, 20.0, &["incoming", "environment"][..]),
                    (Case::Cooldown, 0, 79, 20.0, &["incoming"][..]),
                    (Case::Early, 0, 78, 20.0, &[][..]),
                    (Case::NoStarvation, 1, 79, 20.0, &[][..]),
                ] {
                    *observed.lock().unwrap() = (case, Vec::new());
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| {
                            guest.position = [3.5, 64.0, 5.5];
                            assert!(initialize(game.motion, guest));
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.on_ground = true;
                            guest.health = 20.0;
                            guest.food = food;
                            guest.saturation = 0.0;
                            let motion = guest.motion.as_mut().unwrap();
                            motion.health.food.timer = timer;
                            if case == Case::Cooldown {
                                motion.health.hurt.cooldown = 19;
                                motion.health.hurt.last_hurt = 5.0;
                            }
                        })
                        .unwrap();
                    game.guest_server_tick(12);
                    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert_eq!(&observed.lock().unwrap().1, order, "{case:?}");
                    assert_eq!(guest.health, expected_health, "{case:?}");
                    assert_eq!(guest.food, food, "{case:?}");
                    let motion = guest.motion.as_ref().unwrap();
                    let next_timer = if case == Case::Early { 79 } else { 0 };
                    assert_eq!(motion.health.food.timer, next_timer, "{case:?}");
                }
            });
        });
    });
}
