use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_hopper_pickup_item_pre(
        &self,
        dimension: LevelDimension,
        pos: BlockPos,
        source: [f64; 3],
        item: &str,
        count: u32,
        cancel: &mut bool,
    ) {
        self.fire_hopper_pickup_item_pre(dimension, pos, source, item, count, cancel);
    }

    pub(in crate::game) fn hopper_may_pick_up_item(
        &self,
        pos: BlockPos,
        source: [f64; 3],
        item: &str,
        count: u32,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_hopper_pickup_item_pre(
                self.dimension,
                pos,
                source,
                item,
                count,
                &mut cancel,
            );
        }
        !cancel
    }
}
