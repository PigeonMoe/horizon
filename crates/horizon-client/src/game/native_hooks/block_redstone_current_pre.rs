use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_block_redstone_current_pre(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        old: i32,
        new: i32,
        cancel: &mut bool,
    ) {
        self.fire_block_redstone_current_pre(dimension, pos, old, new, cancel);
    }
}
