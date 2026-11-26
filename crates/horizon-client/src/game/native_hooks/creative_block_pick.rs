use super::*;

impl Game<'_> {
    pub(in crate::game) fn creative_block_pick_override(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        include_data: bool,
    ) -> Option<ItemStack> {
        if !self.creative || !mod_api::__private::listened(mod_api::HookId::CreativeBlockPick) {
            return None;
        }
        let player = self.native_acting_player()?;
        let mut stack = None;
        self.native_creative_block_pick(super::fire::CreativeBlockPickCall {
            player,
            pos,
            state,
            include_data,
            stack: &mut stack,
        });
        stack
    }

    pub(in crate::game) fn native_creative_block_pick(
        &mut self,
        call: super::fire::CreativeBlockPickCall<'_>,
    ) {
        self.fire_creative_block_pick(call);
    }
}
