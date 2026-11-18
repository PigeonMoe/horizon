use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_sponge_absorb_pre(
        &self,
        dimension: LevelDimension,
        sponge: BlockPos,
        pos: BlockPos,
        cancel: &mut bool,
    ) {
        self.fire_sponge_absorb_pre(dimension, sponge, pos, cancel);
    }
}
