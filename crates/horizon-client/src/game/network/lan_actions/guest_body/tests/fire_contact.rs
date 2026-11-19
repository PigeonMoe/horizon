//! Fire contact on the actual server turn, with ordinary protection and live slots.
use super::initialize;
use horizon_client::chunk::Chunk;
use mod_api::{
    ActiveEffect, EffectApplicable, Flow, Hooks, Host, HostTransaction, ModEntry, ServerHost,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Case {
    FirstContact,
    Inventory,
    Ender,
    DeniedHit,
    Cooldown,
    Immune,
    DeniedEffect,
    BurningOnly,
    NoTalisman,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Observed {
    DirectContact,
    Burning,
    Reaction(f32),
    EffectDenied,
}

struct Probe {
    scenario: Arc<Mutex<(Case, Vec<Observed>)>>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn living_incoming_damage(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        event: &mut mod_api::LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        let mut scenario = self.scenario.lock().unwrap();
        match event.source.damage_type {
            "minecraft:in_fire" => {
                assert_eq!((event.source.direct, event.source.causing), (0, 0));
                scenario.1.push(Observed::DirectContact);
                if scenario.0 == Case::DeniedHit {
                    return Flow::Cancel;
                }
            }
            "minecraft:on_fire" => scenario.1.push(Observed::Burning),
            _ => {}
        }
        Flow::Continue
    }

    fn entity_damage_target_pre(
        &self,
        _: &mut dyn mod_api::LevelRandomHost,
        _: &mut mod_api::EntityDamageTargetPreEvent<'_>,
    ) -> Flow {
        panic!("ambient fire must not invent an entity attacker")
    }

    fn mob_effect_applicable(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::MobEffectApplicableEvent,
    ) -> Option<EffectApplicable> {
        let mut scenario = self.scenario.lock().unwrap();
        if scenario.0 == Case::DeniedEffect {
            scenario.1.push(Observed::EffectDenied);
            Some(EffectApplicable::Deny)
        } else {
            None
        }
    }

    fn player_environment_damage_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::PlayerEnvironmentDamagePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.damage_type, "minecraft:in_fire");
        assert_eq!(event.player.uuid(), 12);
        {
            let mut scenario = self.scenario.lock().unwrap();
            let ticks = event.player.entity().fire_ticks(host).unwrap();
            if scenario.0 == Case::FirstContact {
                assert_eq!(ticks, -19);
            } else {
                assert!(ticks > 0);
            }
            scenario.1.push(Observed::Reaction(event.damage));
        }
        let player = event.player;
        let resistance = host.effect_key("minecraft:fire_resistance").unwrap();
        let living = host.living(player.entity()).unwrap();
        let outcome: Result<(), ()> = host.transaction(|host, _| {
            let on_hand = player
                .inventory_slots(host)
                .and_then(|slots| slots.into_iter().find(|(slot, _)| *slot == 2));
            if let Some((slot, stack)) = on_hand {
                player
                    .set_inventory_slot(host, slot, stack.shrink(1))
                    .map_err(|_| ())?;
            } else {
                let (slot, stack) = player
                    .ender_chest(host)
                    .and_then(|slots| slots.into_iter().find(|(slot, _)| *slot == 7))
                    .ok_or(())?;
                player
                    .set_ender_chest_slot(host, slot, stack.shrink(1))
                    .map_err(|_| ())?;
            }
            living
                .add_effect(
                    host,
                    &ActiveEffect::new(resistance, 3600, 4, false, true, true),
                )
                .then_some(())
                .ok_or(())
        });
        if outcome.is_ok() {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn fire_contact_reacts_after_admission_and_consumes_authoritative_storage() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let scenario = Arc::new(Mutex::new((Case::Inventory, Vec::new())));
        let mods = vec![ModEntry::new(
            "fire-contact",
            Box::new(Probe {
                scenario: scenario.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                game.build_registries();
                let args = [
                    (Case::FirstContact, "fire", 1, 0, 20.0, 0, 0, Some(4)),
                    (Case::Inventory, "fire", 2, 2, 20.0, 1, 2, Some(4)),
                    (Case::Ender, "soul_fire", 0, 1, 20.0, 0, 0, Some(4)),
                    (Case::DeniedHit, "fire", 2, 2, 20.0, 2, 2, None),
                    (Case::Cooldown, "fire", 2, 2, 20.0, 2, 2, None),
                    (Case::Immune, "fire", 2, 2, 20.0, 2, 2, Some(0)),
                    (Case::DeniedEffect, "soul_fire", 0, 2, 18.0, 0, 2, None),
                    (Case::DeniedEffect, "soul_fire", 2, 2, 18.0, 2, 2, None),
                    (Case::BurningOnly, "air", 2, 2, 19.0, 2, 2, None),
                    (Case::NoTalisman, "fire", 0, 0, 19.0, 0, 0, None),
                ];
                let player = game.player(12).unwrap();
                for (case, source, main, ender, hp, main_left, ender_left, amp) in args {
                    *scenario.lock().unwrap() = (case, Vec::new());
                    let registry = game.pack.assets.registry;
                    let base = if source == "soul_fire" {
                        "minecraft:soul_soil"
                    } else {
                        "minecraft:netherrack"
                    };
                    let ground = registry.default_state(base).unwrap();
                    let hazard = registry
                        .default_state(&format!("minecraft:{source}"))
                        .unwrap();
                    let mut chunk = Chunk::empty(0, 0);
                    for x in 0..16 {
                        for z in 0..16 {
                            chunk.set(x, 63, z, ground.0);
                        }
                    }
                    chunk.set(3, 64, 5, hazard.0);
                    game.world.chunks.insert((0, 0), Arc::new(chunk));
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
                            guest.effects.clear();
                            let motion = guest.motion.as_mut().unwrap();
                            motion.fire_ticks = match case {
                                Case::FirstContact => -20,
                                Case::BurningOnly => 40,
                                _ => -1,
                            };
                            if case == Case::Cooldown {
                                motion.health.hurt.cooldown = 19;
                                motion.health.hurt.last_hurt = 5.0;
                            }
                        })
                        .unwrap();
                    if case == Case::Immune {
                        let effect = game.effect_key("minecraft:fire_resistance").unwrap();
                        let living = game.living(player.entity()).unwrap();
                        assert!(living.add_effect(
                            game,
                            &ActiveEffect::new(effect, 120, 0, false, true, true),
                        ));
                    }
                    let token = |host: &dyn Host, count| {
                        (count > 0).then(|| {
                            host.item_from_snbt(&format!(
                                r#"{{id:"minecraft:emerald",count:{count},components:{{"minecraft:custom_data":{{proof:1}}}}}}"#
                            ))
                            .unwrap()
                        })
                    };
                    let hand = token(game, main);
                    let chest = token(game, ender);
                    player.set_inventory_slot(game, 2, hand).unwrap();
                    player.set_ender_chest_slot(game, 7, chest).unwrap();
                    game.guest_server_tick(12);
                    let expected_order: &[Observed] = match case {
                        Case::FirstContact | Case::Inventory | Case::NoTalisman => {
                            &[Observed::DirectContact, Observed::Reaction(1.0)]
                        }
                        Case::Ender => &[Observed::DirectContact, Observed::Reaction(2.0)],
                        Case::DeniedEffect => &[
                            Observed::DirectContact,
                            Observed::Reaction(2.0),
                            Observed::EffectDenied,
                        ],
                        Case::DeniedHit | Case::Cooldown => &[Observed::DirectContact],
                        Case::Immune => &[],
                        Case::BurningOnly => &[Observed::Burning],
                    };
                    assert_eq!(scenario.lock().unwrap().1, expected_order, "{case:?}");
                    let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                    assert_eq!(guest.health, hp, "{case:?}");
                    assert_eq!(guest.effects.amplifier("fire_resistance"), amp, "{case:?}");
                    if amp == Some(4) {
                        let effect = guest.effects.get("fire_resistance").unwrap();
                        assert!((3599..=3600).contains(&effect.duration));
                        assert!(!effect.ambient && effect.visible && effect.show_icon);
                    }
                    let ticks = guest.motion.as_ref().unwrap().fire_ticks;
                    if case == Case::FirstContact {
                        assert_eq!(ticks, -19);
                    } else {
                        assert!(ticks > 0, "{case:?}");
                    }
                    let hand = player
                        .inventory_slots(game)
                        .unwrap()
                        .into_iter()
                        .find(|(slot, _)| *slot == 2)
                        .map(|(_, stack)| stack);
                    let chest = player
                        .ender_chest(game)
                        .unwrap()
                        .into_iter()
                        .find(|(slot, _)| *slot == 7)
                        .map(|(_, stack)| stack);
                    assert_eq!(hand, token(game, main_left), "{case:?} inventory");
                    assert_eq!(chest, token(game, ender_left), "{case:?} ender chest");
                }
            });
        });
    });
}
