use super::initialize;
use horizon_client::chunk::Chunk;
use mod_api::{ActiveEffect, Flow, Hooks, Host, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Cancel,
    Continue,
    BootsAllow,
    Boots,
    SlowFalling,
    Denied,
    Cooldown,
}

struct Probe(Arc<Mutex<(Case, Vec<f32>, usize)>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        if event.source.damage_type == "minecraft:fall" && self.0.lock().unwrap().0 == Case::Denied
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn player_fall_reaction(
        &self,
        _: &dyn mod_api::LoopContext,
        event: &mut mod_api::PlayerFallReactionEvent<'_>,
    ) -> Flow {
        let mut state = self.0.lock().unwrap();
        state.2 += 1;
        if state.0 == Case::BootsAllow {
            return Flow::Continue;
        }
        assert_eq!(state.0, Case::Boots);
        *event.effect = (1, None);
        Flow::Cancel
    }

    fn player_fall_reaction_post(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerFallReactionPostEvent,
    ) {
        assert_eq!((event.player.uuid(), event.reaction), (12, 1));
        assert_eq!(self.0.lock().unwrap().0, Case::Boots);
    }

    fn player_environment_damage_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerEnvironmentDamagePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.damage_type, "minecraft:fall");
        assert_eq!(event.player.uuid(), 12);
        assert_eq!(event.player.entity().position(host), Some([3.0, 64.0, 5.0]));
        assert_eq!(event.player.inventory_slots(host).unwrap()[0].0, 2);
        assert_eq!(event.player.ender_chest(host).unwrap()[0].0, 7);
        let mut state = self.0.lock().unwrap();
        state.1.push(event.damage);
        if state.0 == Case::Cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn fall_reaction_reads_the_real_barefoot_guest_after_protection_and_equipment() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let state = Arc::new(Mutex::new((Case::Cancel, Vec::new(), 0)));
        let mods = vec![ModEntry::new(
            "fall-contact",
            Box::new(Probe(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                let mut chunk = Chunk::empty(0, 0);
                for x in 0..16 {
                    for z in 0..16 {
                        chunk.set(x, 63, z, stone.0);
                    }
                }
                game.world.chunks.insert((0, 0), Arc::new(chunk));
                let player = game.player(12).unwrap();
                let host: &dyn Host = game;
                let token = host
                    .item_from_snbt(r#"{id:"minecraft:emerald",count:1}"#)
                    .unwrap();
                let boots = host
                    .item_from_snbt(r#"{id:"minecraft:leather_boots",count:1}"#)
                    .unwrap();
                player
                    .set_inventory_slot(game, 2, Some(token.clone()))
                    .unwrap();
                player.set_ender_chest_slot(game, 7, Some(token)).unwrap();
                for (case, expected_health, expected_calls) in [
                    (Case::Cancel, 20.0, 1),
                    (Case::Continue, 13.0, 1),
                    (Case::BootsAllow, 13.0, 1),
                    (Case::Boots, 20.0, 0),
                    (Case::SlowFalling, 20.0, 0),
                    (Case::Denied, 20.0, 0),
                    (Case::Cooldown, 20.0, 0),
                ] {
                    *state.lock().unwrap() = (case, Vec::new(), 0);
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| {
                            guest.position = [3.2, 64.0, 5.0];
                            guest.health = 20.0;
                            guest.effects.clear();
                            guest.motion = None;
                            assert!(initialize(game.motion, guest));
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.on_ground = true;
                            guest.moves_from_ground = false;
                            guest.moves = vec![
                                ([0.0, -9.0, 0.0], false),
                                ([0.0, -1.0, 0.0], true),
                                ([0.2, 0.0, 0.0], true),
                            ];
                            if case == Case::Cooldown {
                                let hurt = &mut guest.motion.as_mut().unwrap().health.hurt;
                                hurt.cooldown = 19;
                                hurt.last_hurt = 8.0;
                            }
                        })
                        .unwrap();
                    player
                        .set_inventory_slot(
                            game,
                            36,
                            matches!(case, Case::Boots | Case::BootsAllow).then(|| boots.clone()),
                        )
                        .unwrap();
                    if case == Case::SlowFalling {
                        let slow = game.effect_key("minecraft:slow_falling").unwrap();
                        let living = game.living(player.entity()).unwrap();
                        assert!(
                            living.add_effect(
                                game,
                                &ActiveEffect::new(slow, 100, 0, false, true, true)
                            )
                        );
                    }
                    game.guest_server_tick(12);
                    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    let result = state.lock().unwrap();
                    assert_eq!(guest.health, expected_health, "{result:?}");
                    assert_eq!(guest.position, [3.2, 64.0, 5.0]);
                    assert_eq!(result.1, vec![7.0; expected_calls], "{case:?}");
                    assert_eq!(
                        result.2,
                        usize::from(matches!(case, Case::Boots | Case::BootsAllow)),
                        "{case:?}"
                    );
                }

                // A command uses FALL only when it supplies no direct damager.
                game.set_allow_commands(true);
                for (case, by, expected_hurt, expected_calls) in [
                    (Case::Cancel, "", false, 1),
                    (Case::Continue, "", true, 1),
                    (Case::Boots, "", false, 0),
                    (Case::Denied, "", false, 0),
                    (Case::Cancel, " by guest13", true, 0),
                ] {
                    *state.lock().unwrap() = (case, Vec::new(), 0);
                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .update_guest(12, |guest| {
                            guest.position = [3.0, 64.0, 5.0];
                            guest.health = 20.0;
                            guest.motion = None;
                            assert!(initialize(game.motion, guest));
                        })
                        .unwrap();
                    player
                        .set_inventory_slot(game, 36, (case == Case::Boots).then(|| boots.clone()))
                        .unwrap();
                    let command = format!("damage guest12 7 minecraft:fall{by}");
                    assert_eq!(
                        game.run_command(&command),
                        expected_hurt,
                        "{case:?} {command}"
                    );
                    let hp = game.lan.as_ref().unwrap().hub.guest(12).unwrap().health;
                    assert_eq!(hp, if expected_hurt { 13.0 } else { 20.0 }, "{command}");
                    assert_eq!(
                        state.lock().unwrap().1,
                        vec![7.0; expected_calls],
                        "{case:?}"
                    );
                }
            });
        });
    });
}
