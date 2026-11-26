//! The replacement item for a creative block pick.

pub trait Hooks {
    /// A creative player picked a loaded block within reach. Return the complete
    /// item to pick after the block's own clone query. The game does not copy
    /// block entity data onto a replacement, even when `include_data` is true.
    /// Return None for normal picking, including vanilla Ctrl+Pick data copying.
    /// Read the player and block without changing either. The ordinary pick
    /// cancellation still runs before the game selects or writes any slot.
    #[hook(
        side = "server",
        contract = "replace",
        out = "stack",
        paper = "InventoryCreativeEvent"
    )]
    fn creative_block_pick(
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
        include_data: bool,
        stack: &mut Option<ItemStack>,
    );
}
