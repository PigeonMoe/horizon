//! Real server fluid hazards reach reactions after admission and before health changes.
use super::initialize;
use horizon_client::chunk::Chunk;
use mod_api::{ActiveEffect, Flow, Hooks, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    Contact,
    Unprotected,
    Denied,
    Cooldown,
    Immune,
    Command,
    SourcedCommand,
}

struct Probe(Arc<Mutex<(Case, Vec<f32>)>>);

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        if matches!(
            event.source.damage_type,
            "minecraft:lava" | "minecraft:drown"
        ) && self.0.lock().unwrap().0 == Case::Denied
        {
            return Flow::Cancel;
        }
        Flow::Continue
    }

    fn player_environment_damage_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerEnvironmentDamagePreEvent<'_>,
    ) -> Flow {
        let effect = match event.damage_type {
            "minecraft:lava" => "minecraft:fire_resistance",
            "minecraft:drown" => "minecraft:water_breathing",
            _ => return Flow::Continue,
        };
        assert_eq!(event.player.uuid(), 12);
        let entity = event.player.entity();
        let mut state = self.0.lock().unwrap();
        if matches!(state.0, Case::Contact | Case::Unprotected) {
            if event.damage_type == "minecraft:lava" {
                assert!(entity.fire_ticks(host).unwrap() > 0);
            } else {
                assert_eq!(entity.air_supply(host), Some(0));
            }
        }
        state.1.push(event.damage);
        let protected = state.0 != Case::Unprotected;
        drop(state);

        if !protected {
            return Flow::Continue;
        }
        let key = host.effect_key(effect).unwrap();
        assert!(
            host.living(entity)
                .unwrap()
                .add_effect(host, &ActiveEffect::new(key, 3600, 4, true, true, true),)
        );
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn lava_and_drowning_observe_admitted_guest_damage_and_preserve_contact() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let state = Arc::new(Mutex::new((Case::Contact, Vec::new())));
        let mods = vec![ModEntry::new(
            "liquid-contact",
            Box::new(Probe(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                let player = game.player(12).unwrap();
                for (fluid, source, effect, amount) in [
                    ("water", "minecraft:drown", "water_breathing", 2.0),
                    ("lava", "minecraft:lava", "fire_resistance", 4.0),
                    ("lava_cauldron", "minecraft:lava", "fire_resistance", 4.0),
                ] {
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let hazard = registry
                        .default_state(&format!("minecraft:{fluid}"))
                        .unwrap();
                    for case in [
                        Case::Contact,
                        Case::Unprotected,
                        Case::Denied,
                        Case::Cooldown,
                        Case::Immune,
                        Case::Command,
                        Case::SourcedCommand,
                    ] {
                        *state.lock().unwrap() = (case, Vec::new());
                        let command = matches!(case, Case::Command | Case::SourcedCommand);
                        let cauldron = fluid == "lava_cauldron";
                        if cauldron && command {
                            continue;
                        }
                        let mut chunk = Chunk::empty(0, 0);
                        for x in 0..16 {
                            for z in 0..16 {
                                chunk.set(x, 63, z, stone.0);
                            }
                        }
                        if !command {
                            for y in 64..=if cauldron { 64 } else { 66 } {
                                chunk.set(3, y, 5, hazard.0);
                            }
                        }
                        game.world.chunks.insert((0, 0), Arc::new(chunk));
                        game.lan
                            .as_mut()
                            .unwrap()
                            .hub
                            .update_guest(12, |guest| {
                                guest.position = [3.5, if cauldron { 64.25 } else { 64.0 }, 5.5];
                                guest.motion = None;
                                assert!(initialize(game.motion, guest));
                                guest.loaded = true;
                                guest.load_timeout = 0;
                                guest.health = 20.0;
                                guest.effects.clear();
                                let motion = guest.motion.as_mut().unwrap();
                                motion.air = -19;
                                motion.fire_ticks = -1;
                                if case == Case::Cooldown {
                                    motion.health.hurt.cooldown = 19;
                                    motion.health.hurt.last_hurt = 5.0;
                                }
                            })
                            .unwrap();
                        if case == Case::Immune {
                            let key = game.effect_key(&format!("minecraft:{effect}")).unwrap();
                            let living = game.living(player.entity()).unwrap();
                            assert!(living.add_effect(
                                game,
                                &ActiveEffect::new(key, 120, 0, false, true, true),
                            ));
                        }
                        if command {
                            let by = if case == Case::SourcedCommand {
                                " by guest13"
                            } else {
                                ""
                            };
                            assert_eq!(
                                game.run_command(&format!("damage guest12 {amount} {source}{by}")),
                                case == Case::SourcedCommand,
                            );
                        } else {
                            game.guest_server_tick(12);
                        }
                        let protected = matches!(case, Case::Contact | Case::Command);
                        let admitted = protected || case == Case::Unprotected;
                        assert_eq!(
                            state.lock().unwrap().1,
                            vec![amount; usize::from(admitted)],
                            "{fluid} {case:?}"
                        );
                        let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                        let hp = if matches!(case, Case::SourcedCommand | Case::Unprotected) {
                            20.0 - amount
                        } else {
                            20.0
                        };
                        assert_eq!(guest.health, hp, "{fluid} {case:?}");
                        if protected {
                            let applied = guest.effects.get(effect).unwrap();
                            assert_eq!(guest.effects.amplifier(effect), Some(4));
                            assert!((3599..=3600).contains(&applied.duration));
                            assert!(applied.ambient && applied.visible && applied.show_icon);
                        } else if case == Case::Immune {
                            assert_eq!(guest.effects.amplifier(effect), Some(0));
                        } else {
                            assert!(guest.effects.get(effect).is_none());
                        }
                    }
                }
            });
        });
    });
}
