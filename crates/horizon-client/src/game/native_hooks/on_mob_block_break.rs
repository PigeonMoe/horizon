use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_on_mob_block_break(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        entity: u32,
        kind: &str,
    ) {
        self.fire_on_mob_block_break(pos, state, entity, kind);
    }

    /// Tells the part after the break is admitted and checks whether it
    /// replaced the block. With no part, the normal break can continue.
    pub(in crate::game) fn mob_break_replaced(
        &mut self,
        mob: u32,
        pos: [i32; 3],
        state: BlockStateId,
    ) -> bool {
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::OnMobBlockBreak,
                state,
            )
        {
            return false;
        }
        let Some(kind) = self.mobs.iter().find(|m| m.id == mob).map(mob_kind) else {
            return false;
        };
        let [x, y, z] = pos;
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_on_mob_block_break(BlockPos::new(x, y, z), state, mob, &kind);
        }) {
            log::warn!("on_mob_block_break callback failed: {error}");
        }
        self.world.block(x, y, z) != state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{creatures::SpawnSite, native_entity_refs::tests::with_game};
    use horizon_client::{chunk::Chunk, mobs::MobEvent};
    use horizon_core::position::ChunkPos;
    use mod_api::{
        BlockPart, Flow, Hooks, LivingDestroyBlockEvent, OnMobBlockBreakEvent, Registrar,
        RegistrationContext, ServerHost,
    };

    const MARKED: [i32; 3] = [1, 64, 1];
    const PLAIN: [i32; 3] = [3, 64, 1];
    const DENIED: [i32; 3] = [5, 64, 1];

    struct Chest;
    #[mod_api::horizon_part]
    impl BlockPart for Chest {
        fn on_mob_block_break(
            &self,
            host: &mut dyn ServerHost,
            event: &mut OnMobBlockBreakEvent<'_>,
        ) {
            let pos = [event.pos.x, event.pos.y, event.pos.z];
            if event.kind == "minecraft:wither" && pos != PLAIN {
                let level = host.level(host.dimension()).unwrap();
                let air = host.block_state_id("minecraft:air").unwrap();
                assert_eq!(
                    level.set_block(
                        host,
                        pos,
                        air,
                        BlockUpdateFlags::NEIGHBORS
                            | BlockUpdateFlags::CLIENTS
                            | BlockUpdateFlags::SKIP_BLOCK_ENTITY_SIDE_EFFECTS,
                    ),
                    Ok(true)
                );
            }
        }
    }

    struct Protection;
    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn register(&self, registry: &mut dyn Registrar, _: &RegistrationContext) {
            let chest = registry.block_key("minecraft:chest");
            registry.block_part(chest, Chest);
        }

        fn living_destroy_block(
            &self,
            _: &mut dyn ServerHost,
            event: &mut LivingDestroyBlockEvent<'_>,
        ) -> Flow {
            if event.pos == DENIED {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn direct_wither_block_break_respects_veto_and_chest_replacement() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("wither_chest", Box::new(Protection))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                let chest = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:chest")
                    .unwrap();
                for [x, y, z] in [MARKED, PLAIN, DENIED] {
                    game.level_set([x, y, z], chest).unwrap();
                    let mut contents = vec![None; 27];
                    contents[0] = Some(ItemStack::new("minecraft:diamond", 3));
                    game.world
                        .block_entities
                        .insert((x, y, z), BlockEntity::Chest(contents));
                }
                let wither = game
                    .spawn_mob(
                        "minecraft:wither",
                        [3.5, 66.0, 2.5],
                        false,
                        SpawnSite::Plain,
                    )
                    .unwrap();
                for pos in [MARKED, PLAIN, DENIED] {
                    game.handle_mob_events(vec![MobEvent::BreakBlock {
                        pos,
                        drop: true,
                        by: Some(wither),
                        breaker: None,
                    }])
                    .unwrap();
                }
                assert_eq!(
                    game.world.block(MARKED[0], MARKED[1], MARKED[2]),
                    game.world.air
                );
                assert_eq!(
                    game.world.block(PLAIN[0], PLAIN[1], PLAIN[2]),
                    game.world.air
                );
                assert_eq!(game.world.block(DENIED[0], DENIED[1], DENIED[2]), chest);
                assert!(!game.world.block_entities.contains_key(&(1, 64, 1)));
                assert!(!game.world.block_entities.contains_key(&(3, 64, 1)));
                let Some(BlockEntity::Chest(contents)) = game.world.block_entities.get(&(5, 64, 1))
                else {
                    panic!("the protected chest must keep its inventory");
                };
                assert_eq!(contents[0].as_ref().unwrap().count, 3);
                let mut drops = game
                    .dropped
                    .iter()
                    .map(|drop| (&*drop.stack.item, drop.stack.count))
                    .collect::<Vec<_>>();
                drops.sort_unstable();
                assert_eq!(drops, [("minecraft:chest", 1), ("minecraft:diamond", 3)]);
            });
        });
    }
}
