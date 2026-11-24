//! Drops supplied by mods for a successful creative block break.

pub trait Hooks {
    /// A creative player successfully broke a block at `pos` with the given `state` and `tool`.
    /// The drop list starts empty and the experience starts at zero. A listener can add item
    /// entities or experience without evaluating a vanilla loot table. The drop list excludes
    /// the block's own removal side effects, such as a filled shulker box preserving its contents.
    /// The removed block entity's saved data and attachments are readable through `block_entity`
    /// for the duration of the event. Cancelling suppresses the new drops and experience.
    /// The listener-supplied items and experience are explicit spawns, independent of the
    /// vanilla `block_drops` game rule. Without a listener, the break creates no additional drops.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client::game::native_hooks::block_drops::GameLevel<'_>::native_creative_block_drops"
    )]
    fn creative_block_drops(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        breaker: Option<u128>,
        breaker_entity: Option<EntityRef>,
        block_entity: Option<BlockEntityRef>,
        tool: Option<&ItemStack>,
        drops: &mut Vec<ItemDrop>,
        experience: &mut u32,
        cancel: &mut bool,
    );
}
