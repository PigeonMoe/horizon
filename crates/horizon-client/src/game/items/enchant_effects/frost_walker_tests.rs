use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
use crate::game::sculk::TEST_POSTED_EVENTS;
use horizon_client::{chunk::Chunk, items::ItemStack, mobs::Who, motion::PlayerMotion};
use horizon_core::geometry::Vec3;
use mod_api::{BlockFormEvent, Flow, Hooks, LevelHost, ModEntry};
use std::sync::{Arc, Mutex};

const PROTECTED: [i32; 3] = [7, 63, 8];
const OPEN: [i32; 3] = [9, 63, 8];

struct Listener(Arc<Mutex<Vec<[i32; 3]>>>);

#[mod_api::__subscriptions]
impl Hooks for Listener {
    fn block_form(&self, host: &dyn LevelHost, event: &mut BlockFormEvent) -> Flow {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        assert!(
            host.state_block(event.from)
                .is_some_and(|id| id == "minecraft:water")
        );
        assert!(
            host.state_block(event.to)
                .is_some_and(|id| id == "minecraft:frosted_ice")
        );
        self.0.lock().unwrap().push(pos);
        if pos == PROTECTED {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn frost_walker_leaves_refused_source_water_and_places_the_neighbor() {
    let candidates = Arc::new(Mutex::new(Vec::new()));
    let mods = Arc::from([ModEntry::new(
        "frost-walker-form-test",
        Box::new(Listener(candidates.clone())),
    )]);
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.build_registries();
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = &game.pack.assets.registry;
            let water = registry.default_state("minecraft:water").unwrap();
            let ice = registry.default_state("minecraft:frosted_ice").unwrap();
            let stone = registry.default_state("minecraft:stone").unwrap();
            let floor = [8, 63, 8];
            game.world.set(floor[0], floor[1], floor[2], stone).unwrap();
            for [x, y, z] in [PROTECTED, OPEN] {
                game.world.set(x, y, z, water).unwrap();
            }

            let at = [8.5, 64.0, 8.5];
            let mut player =
                PlayerMotion::new(game.motion, Vec3::new(at[0], at[1], at[2])).unwrap();
            player.ground.on_ground = true;
            game.player = Some(player);
            bind_local_player(game, 80, 81);
            let mut boots = ItemStack::new("minecraft:diamond_boots", 1);
            boots
                .data_mut()
                .enchantments
                .push(("minecraft:frost_walker".into(), 1));
            game.armor[3] = Some(boots);

            TEST_POSTED_EVENTS.with(|events| *events.borrow_mut() = Some(Vec::new()));
            game.location_changed_effects(at, false);
            let events = TEST_POSTED_EVENTS.with(|events| events.borrow_mut().take().unwrap());

            assert_eq!(*candidates.lock().unwrap(), [PROTECTED, OPEN]);
            assert_eq!(
                game.world.block(PROTECTED[0], PROTECTED[1], PROTECTED[2]),
                water
            );
            assert_eq!(game.world.block(OPEN[0], OPEN[1], OPEN[2]), ice);
            assert_eq!(game.world.block(floor[0], floor[1], floor[2]), stone);
            assert_eq!(
                events,
                [("minecraft:block_place", [9.5, 63.5, 8.5], Some(Who::Player))]
            );
        });
    });
}
