use super::*;
use crate::game::native_entity_refs::tests::{run_on_guest_stack, with_game};
use horizon_client::mobs::goals::{Cx, EatBlock, Goal};
use mod_api::{EntityChangeBlockPreEvent, Flow, Hooks, ModEntry, ServerHost};
use std::sync::Mutex;

const PROTECTED_X: i32 = 3;
const OPEN_X: i32 = 8;

struct Listener(Arc<Mutex<Vec<([i32; 3], u32)>>>);

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn entity_change_block_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityChangeBlockPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.player, None);
        let (kind, from, to) = match event.pos[2] {
            4 => ("minecraft:zombie", "minecraft:powder_snow", "minecraft:air"),
            8 => ("minecraft:sheep", "minecraft:short_grass", "minecraft:air"),
            12 => ("minecraft:sheep", "minecraft:grass_block", "minecraft:dirt"),
            _ => panic!("unexpected block change at {:?}", event.pos),
        };
        assert_eq!(event.kind, kind);
        assert_eq!(Some(event.from), host.block_state_id(from));
        assert_eq!(Some(event.to), host.block_state_id(to));
        self.0
            .lock()
            .unwrap()
            .push((event.pos, event.entity.unwrap()));
        if event.pos[0] == PROTECTED_X {
            Flow::Cancel
        } else {
            assert_eq!(event.pos[0], OPEN_X);
            Flow::Continue
        }
    }
}

fn set(game: &mut Game<'_>, [x, y, z]: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(x, y, z, state).unwrap();
}

fn name(game: &Game<'_>, [x, y, z]: [i32; 3]) -> String {
    game.res
        .facts()
        .get(game.world.block(x, y, z))
        .name
        .to_string()
}

fn graze(game: &mut Game<'_>, id: u32) {
    let index = game.mobs.iter().position(|mob| mob.id == id).unwrap();
    let mut sheep = game.mobs[index].clone();
    sheep.extra.sheared = true;
    let mut events = Vec::new();
    {
        let w = game.mob_world_base(&[]);
        let mut level = horizon_client::health::TestLevel::new(0);
        let mut cx = Cx {
            w: &w,
            level: &mut level.level(),
            events: &mut events,
        };
        let mut goal = EatBlock::default();
        // Run one whole grazing animation without waiting for its random start.
        goal.start(&mut sheep, &mut cx);
        while goal.can_continue(&mut sheep, &mut cx) {
            goal.tick(&mut sheep, &mut cx);
        }
    }
    game.mobs.update(index, |mob| *mob = sheep);
    game.handle_mob_events(events).unwrap();
    assert!(!game.mobs[index].extra.sheared, "grazing regrows wool");
}

#[test]
#[ignore = "daily"]
fn living_block_changes_keep_powder_snow_and_grass_when_denied() {
    run_on_guest_stack(|| {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "living-block-protection",
            Box::new(Listener(seen.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.load_ticking_neighbourhood(0, 0);
                assert!(game.run_command("forceload add 0 0"));
                game.world.day_time = 18000;
                let mut expected = Vec::new();
                for x in [PROTECTED_X, OPEN_X] {
                    for (z, kind) in [(4, "zombie"), (8, "sheep"), (12, "sheep")] {
                        set(
                            game,
                            [x, 64, z],
                            if z == 4 {
                                "minecraft:stone"
                            } else {
                                "minecraft:grass_block"
                            },
                        );
                        set(
                            game,
                            [x, 65, z],
                            match z {
                                4 => "minecraft:powder_snow",
                                8 => "minecraft:short_grass",
                                _ => "minecraft:air",
                            },
                        );
                        let id = game
                            .spawn_mob_spawned(
                                kind,
                                [f64::from(x) + 0.5, 65.0, f64::from(z) + 0.5],
                                SpawnSite::Raw,
                            )
                            .unwrap();
                        let index = game.mobs.iter().position(|mob| mob.id == id).unwrap();
                        expected.push(([x, if z == 12 { 64 } else { 65 }, z], id));
                        if z == 4 {
                            game.mobs.update(index, |mob| mob.ignite_for_ticks(100));
                        } else {
                            graze(game, id);
                        }
                    }
                }
                game.world.game_time += 1;
                game.update_chunk_tickets();
                game.tick_creature_entities().unwrap();
                for mob in game
                    .mobs
                    .iter()
                    .filter(|mob| mob.species() == Species::Zombie)
                {
                    assert!(!mob.is_on_fire(), "powder snow puts out the fire");
                }
                for (z, y, protected, open) in [
                    (4, 65, "minecraft:powder_snow", "minecraft:air"),
                    (8, 65, "minecraft:short_grass", "minecraft:air"),
                    (12, 64, "minecraft:grass_block", "minecraft:dirt"),
                ] {
                    assert_eq!(name(game, [PROTECTED_X, y, z]), protected);
                    assert_eq!(name(game, [OPEN_X, y, z]), open);
                }
                let mut seen = seen.lock().unwrap();
                seen.sort_unstable();
                expected.sort_unstable();
                assert_eq!(*seen, expected);
            });
        });
    });
}
