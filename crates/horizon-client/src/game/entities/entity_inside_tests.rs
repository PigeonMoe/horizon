//! `Entity.applyEffectsFromBlocks` runs in the entity's own tick, and
//! `ServerLevel.tick` ticks only the entities in entity-ticking chunks: a mob
//! past the simulation distance presses no plate, however long it stands on it.

use super::Game;
use super::native_entity_refs::tests::with_game;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::Mob;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

#[test]
#[ignore = "daily"]
fn a_boat_breaks_a_lily_pad_only_when_its_block_change_is_allowed() {
    use mod_api::{Flow, Hooks, Host, MobBlockChangePreEvent, ModEntry, Mods};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    struct Guard {
        deny: Arc<AtomicBool>,
        heard: Arc<Mutex<Vec<u32>>>,
    }
    #[mod_api::__subscriptions]
    impl Hooks for Guard {
        fn mob_block_change_pre(
            &self,
            _: &dyn Host,
            event: &mut MobBlockChangePreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.kind, "minecraft:oak_boat");
            assert_eq!(event.pos, [8, 64, 8]);
            self.heard.lock().unwrap().push(event.entity);
            if self.deny.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    let deny = Arc::new(AtomicBool::new(true));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: Mods = vec![ModEntry::new(
        "lily-guard",
        Box::new(Guard {
            deny: deny.clone(),
            heard: heard.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.load_ticking_neighbourhood(0, 0);
            let registry = game.pack.assets.registry;
            let water = registry.default_state("minecraft:water").unwrap();
            let lily = registry.default_state("minecraft:lily_pad").unwrap();
            game.world.set(8, 63, 8, water).unwrap();
            game.world.set(8, 64, 8, lily).unwrap();
            game.boats.push(super::boats::Boat::new(
                20,
                "oak".into(),
                Vec3::new(8.5, 64.0, 8.5),
                0.0,
            ));
            game.tick_blocks().unwrap();
            assert_eq!(game.world.block(8, 64, 8), lily);
            assert_eq!(*heard.lock().unwrap(), [20]);
            deny.store(false, Ordering::Relaxed);
            game.tick_blocks().unwrap();
            assert_ne!(game.world.block(8, 64, 8), lily);
            assert_eq!(*heard.lock().unwrap(), [20, 20]);
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_mob_presses_a_plate_only_inside_the_simulation_distance() {
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.settings.simulation_distance = 4;
        game.settings.render_distance = 12;
        let registry = game.pack.assets.registry;
        let stone = registry.default_state("minecraft:stone").unwrap();
        let plate = registry
            .default_state("minecraft:stone_pressure_plate")
            .unwrap();
        let zombie = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        // The player in chunk (0, 0); a zombie on a plate in chunk (2, 0), inside
        // the simulation distance, and another in chunk (8, 0): loaded and in
        // view, but past it.
        game.player = Some(
            horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5))
                .unwrap(),
        );
        for (cx, x, id) in [(2, 40, 1), (8, 136, 2)] {
            game.world
                .chunks
                .insert((cx, 0), Arc::new(Chunk::empty(cx, 0)));
            game.load_ticking_neighbourhood(cx, 0);
            game.world.set(x, 63, 8, stone).unwrap();
            game.world.set(x, 64, 8, plate).unwrap();
            game.mobs.push(Mob::new(
                id,
                zombie.clone(),
                Vec3::new(f64::from(x) + 0.5, 64.0, 8.5),
                0.0,
            ));
        }
        game.update_chunk_tickets();
        let ticking = game.entity_ticking_area();
        assert!(ticking(40, 8), "chunk (2, 0) ticks entities");
        assert!(
            !ticking(136, 8),
            "chunk (8, 0) is past the simulation distance"
        );
        for _ in 0..3 {
            game.tick_blocks().unwrap();
        }
        let powered = |game: &Game<'_>, x: i32| {
            let state = game.world.block(x, 64, 8);
            registry
                .state(state)
                .unwrap()
                .properties
                .get("powered")
                .cloned()
        };
        assert_eq!(powered(game, 40).as_deref(), Some("true"));
        assert_eq!(powered(game, 136).as_deref(), Some("false"));
    });
}
