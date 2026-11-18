use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_falling_block_change_pre(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        origin_dimension: &str,
        origin_pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_falling_block_change_pre(dimension, pos, origin_dimension, origin_pos, cancel);
    }
}
