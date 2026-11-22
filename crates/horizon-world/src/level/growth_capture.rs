//! Capture live growth candidates before applying land policies. Reads during
//! growth see the captured edits, even when a listener later refuses one.
use super::{Behaviours, LevelAccess, Sim};
use crate::registry::BlockStateId;
use horizon_core::{nbt::Compound, position::BlockPos};
use std::collections::HashMap;

/// Snapshot of the player using bone meal, taken before lending out the world.
#[derive(Clone, Copy, Debug)]
pub struct GrowthPlayer<'a> {
    pub uuid: u128,
    pub name: &'a str,
    pub permission_level: u8,
}

#[derive(Default)]
pub(super) struct GrowthCapture {
    structure: bool,
    index: HashMap<BlockPos, usize>,
    writes: Vec<Write>,
}

struct Write {
    pos: BlockPos,
    state: BlockStateId,
    // None means the feature writes raw chunk section data.
    update: Option<(i32, i32)>,
    block_entity: Option<BlockEntityWrite>,
    destroyed: Option<(BlockStateId, bool)>,
}

pub(super) enum BlockEntityWrite {
    Load(Compound),
    CreateSpawner(Compound),
}

impl GrowthCapture {
    pub(super) fn read(&self, pos: BlockPos) -> Option<BlockStateId> {
        Some(self.writes[*self.index.get(&pos)?].state)
    }

    fn write_at(&mut self, pos: BlockPos, state: BlockStateId) -> &mut Write {
        let slot = *self.index.entry(pos).or_insert_with(|| {
            let slot = self.writes.len();
            self.writes.push(Write {
                pos,
                state,
                update: None,
                block_entity: None,
                destroyed: None,
            });
            slot
        });
        &mut self.writes[slot]
    }

    pub(super) fn stage(
        &mut self,
        behaviours: &Behaviours,
        pos: BlockPos,
        state: BlockStateId,
        update: Option<(i32, i32)>,
    ) {
        let write = self.write_at(pos, state);
        if write.block_entity.is_some() && !behaviours.keeps_block_entity(state, write.state) {
            write.block_entity = None;
        }
        write.state = state;
        write.update = update;
    }

    pub(super) fn stage_block_entity(
        &mut self,
        pos: BlockPos,
        state: BlockStateId,
        write: BlockEntityWrite,
    ) {
        let pending = &mut self.write_at(pos, state).block_entity;
        // The dungeon configures only a missing entity. A regular load at
        // the same position can replace an earlier staged load or creation.
        if pending.is_none() || matches!(write, BlockEntityWrite::Load(_)) {
            *pending = Some(write);
        }
    }

    pub(super) fn stage_destruction(&mut self, pos: BlockPos, state: BlockStateId, drops: bool) {
        if let Some(&slot) = self.index.get(&pos) {
            self.writes[slot].destroyed = Some((state, drops));
        }
    }
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    pub(super) fn capture_structure(
        &mut self,
        origin: BlockPos,
        run: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        if let Some(capture) = self.growth_capture.as_mut() {
            capture.structure = true;
            return run(self);
        }
        self.capture_growth(origin, true, false, None, run)
    }

    pub(super) fn capture_fertilizer(
        &mut self,
        origin: BlockPos,
        player: Option<GrowthPlayer<'_>>,
        run: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        self.capture_growth(origin, false, true, player, run)
    }

    fn capture_growth(
        &mut self,
        origin: BlockPos,
        structure: bool,
        fertilized: bool,
        player: Option<GrowthPlayer<'_>>,
        run: impl FnOnce(&mut Self) -> bool,
    ) -> bool {
        if !self.level.has_growth_policy() || self.growth_capture.is_some() {
            return run(self);
        }
        self.growth_capture = Some(Box::new(GrowthCapture {
            structure,
            ..GrowthCapture::default()
        }));
        let result = run(self);
        let GrowthCapture {
            structure, writes, ..
        } = *self.growth_capture.take().expect("growth capture");
        // Check every candidate against the pre-growth world before any accepted
        // write can trigger another block action or change a policy decision.
        let accepted: Vec<_> = writes
            .into_iter()
            .filter(|write| {
                self.level
                    .allow_growth_candidate(origin, write.pos, structure, fertilized, player)
            })
            .collect();
        for write in accepted {
            let destroy = write
                .destroyed
                .filter(|(state, _)| self.state(write.pos) == *state);
            if let Some((old, drops)) = destroy {
                self.destroy_particles(write.pos, old);
                if drops {
                    self.drop_resources(old, write.pos);
                }
            }
            let wrote = match write.update {
                Some((flags, limit)) => self.set_block_limit(write.pos, write.state, flags, limit),
                None => self.level.set_raw(write.pos, write.state).is_some(),
            };
            if wrote && let Some((old, _)) = destroy {
                self.game_event_state(write.pos, "minecraft:block_destroy", old);
            }
            if self.state(write.pos) == write.state
                && let Some(entity) = write.block_entity
            {
                match entity {
                    BlockEntityWrite::Load(tag) => self.level.load_block_entity(write.pos, tag),
                    BlockEntityWrite::CreateSpawner(tag) => {
                        self.level.create_feature_spawner(write.pos, tag)
                    }
                }
            }
        }
        result
    }
}
