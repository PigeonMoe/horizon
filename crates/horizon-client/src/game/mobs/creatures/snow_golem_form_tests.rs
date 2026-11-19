use crate::game::native_entity_refs::tests::{run_on_guest_stack, with_game};
use crate::game::sculk::TEST_POSTED_EVENTS;
use horizon_client::mobs::{MobEvent, Species, Who};
use mod_api::{BlockFormEvent, EntityPlaceEvent, Flow, Hooks, LevelHost, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

const PROTECTED: [i32; 3] = [3, 65, 4];
const OPEN: [i32; 3] = [4, 65, 4];
const GENERIC: [i32; 3] = [7, 65, 4];

#[derive(Default)]
struct Calls {
    forms: Vec<[i32; 3]>,
    placements: Vec<[i32; 3]>,
}

struct Listener(Arc<Mutex<Calls>>);

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn block_form(&self, host: &dyn LevelHost, event: &mut BlockFormEvent) -> Flow {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        let expected = if pos == PROTECTED {
            "minecraft:cave_air"
        } else {
            "minecraft:air"
        };
        assert!(
            host.state_block(event.from)
                .is_some_and(|id| id == expected)
        );
        assert!(
            host.state_block(event.to)
                .is_some_and(|id| id == "minecraft:snow")
        );
        self.0.lock().unwrap().forms.push(pos);
        if pos == PROTECTED {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn entity_place(&self, _: &mut dyn ServerHost, event: &mut EntityPlaceEvent) -> Flow {
        self.0
            .lock()
            .unwrap()
            .placements
            .push([event.pos.x, event.pos.y, event.pos.z]);
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn snow_trail_uses_formation_and_vibrates_only_after_writing() {
    run_on_guest_stack(|| {
        let calls = Arc::new(Mutex::new(Calls::default()));
        let mods = Arc::from([ModEntry::new(
            "snow-form-test",
            Box::new(Listener(calls.clone())),
        )]);
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.load_ticking_neighbourhood(0, 0);
                assert!(game.run_command("forceload add 0 0"));
                let registry = &game.pack.assets.registry;
                let cave_air = registry.default_state("minecraft:cave_air").unwrap();
                let air = registry.default_state("minecraft:air").unwrap();
                let stone = registry.default_state("minecraft:stone").unwrap();
                let snow = registry.default_state("minecraft:snow").unwrap();
                for [x, y, z] in [PROTECTED, OPEN, GENERIC] {
                    game.world.set(x, y - 1, z, stone).unwrap();
                }
                game.world
                    .set(PROTECTED[0], PROTECTED[1], PROTECTED[2], cave_air)
                    .unwrap();
                assert!(game.run_command(
                    "summon snow_golem 4.125 65 4.5 {NoGravity:1b,PersistenceRequired:1b}"
                ));
                let golem = game
                    .mobs
                    .iter()
                    .find(|m| m.species() == Species::SnowGolem)
                    .unwrap()
                    .id;
                game.world.game_time += 1;
                game.update_chunk_tickets();
                assert!(game.entity_ticking_area()(OPEN[0], OPEN[2]));

                TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
                game.tick_creature_entities().unwrap();
                let events = TEST_POSTED_EVENTS.with(|events| events.borrow_mut().take().unwrap());
                let placements: Vec<_> = events
                    .into_iter()
                    .filter(|(name, _, _)| *name == "minecraft:block_place")
                    .collect();
                assert_eq!(
                    placements,
                    [(
                        "minecraft:block_place",
                        [4.5, 65.5, 4.5],
                        Some(Who::Mob(golem))
                    )]
                );
                assert_eq!(calls.lock().unwrap().forms, [PROTECTED, OPEN, PROTECTED]);
                assert!(calls.lock().unwrap().placements.is_empty());
                assert_eq!(
                    game.world.block(PROTECTED[0], PROTECTED[1], PROTECTED[2]),
                    cave_air
                );
                assert_eq!(game.world.block(OPEN[0], OPEN[1], OPEN[2]), snow);

                assert!(game.run_command("summon pig 7.5 65 4.5 {NoAI:1b}"));
                let pig = game
                    .mobs
                    .iter()
                    .find(|m| m.species() == Species::Pig)
                    .unwrap()
                    .id;
                game.handle_mob_events(vec![MobEvent::PlaceBlock {
                    mob: pig,
                    pos: GENERIC,
                    state: stone,
                }])
                .unwrap();
                assert_eq!(calls.lock().unwrap().placements, [GENERIC]);
                assert_eq!(calls.lock().unwrap().forms.len(), 3);
                assert_eq!(game.world.block(GENERIC[0], GENERIC[1], GENERIC[2]), air);
            });
        });
    });
}
