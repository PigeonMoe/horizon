use super::*;

impl GameLevel<'_> {
    /// `Block.dropResources` built the item entities `drops` for the block
    /// `state` broken at `pos` by `breaker` (a player's UUID, none for a
    /// level-side break) with `tool`, and `spawnAfterBreak` would award
    /// `experience` points: NeoForge's `BlockDropsEvent`. Edit the list or the
    /// amount, or set `cancel` for no drops, no experience and no
    /// `spawnAfterBreak` effects. Server-side, authoritative levels only.
    #[inline(always)]
    pub(in crate::game) fn native_block_drops(&self, call: super::fire::BlockDropsCall<'_>) {
        self.fire_block_drops(call);
    }

    pub(in crate::game) fn native_creative_block_drops(
        &self,
        call: super::fire::CreativeBlockDropsCall<'_>,
    ) {
        self.fire_creative_block_drops(call);
    }

    pub(in crate::game) fn dispatch_block_drops(
        &self,
        call: super::fire::BlockDropsCall<'_>,
        creative: bool,
    ) {
        if !creative {
            self.native_block_drops(call);
            return;
        }
        self.native_creative_block_drops(super::fire::CreativeBlockDropsCall {
            dimension: call.dimension,
            pos: call.pos,
            state: call.state,
            breaker: call.breaker,
            breaker_entity: call.breaker_entity,
            block_entity: call.block_entity,
            tool: call.tool,
            drops: call.drops,
            experience: call.experience,
            cancel: call.cancel,
        });
    }
}
