use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_block_physics_pre(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockStateId,
        shapes: bool,
        cancel: &mut bool,
    ) {
        self.fire_block_physics_pre(dimension, pos, state, shapes, cancel);
    }
}
