//! The player block-item placement decision, after vanilla computes the placement.

pub trait Hooks {
    /// The player is about to place `stack` at the authoritative `pos`. The
    /// `clicked_pos` and `face` are from the original click. In-place replacement
    /// can put the block at `clicked_pos` itself. The world still holds the old
    /// block and `state` is the primary state predicted by vanilla placement.
    /// Cancel prevents all placement writes and consumption. Only ItemParts
    /// attached to the stack's item are asked. Runs before `entity_place` and
    /// only for a player's block item, never a mob or dispenser.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        scope = "item",
        key = "stack",
        paper = "BlockPlaceEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::on_block_placed_by::Game<'_>::native_block_item_place_pre"
    )]
    fn block_item_place_pre(
        stack: &ItemStack,
        player: PlayerRef,
        pos: [i32; 3],
        clicked_pos: [i32; 3],
        state: BlockState,
        face: BlockFace,
        canceled: &mut bool,
    );
}
