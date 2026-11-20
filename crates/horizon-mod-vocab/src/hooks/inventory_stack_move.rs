//! The item stack selected for an automatic inventory transfer.

pub trait Hooks {
    /// Checks the full selected stack before a hopper, dropper or a mod
    /// calling `LevelRef::allow_inventory_transfer` moves it between inventories.
    /// `stack` is the selection before extraction, including all components.
    /// A refusal leaves the item in its source and skips the destination write.
    /// An earlier veto from `inventory_move_item` also prevents the transfer.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        isolate = true,
        neoforge = "none (component-aware inventory transfer veto)",
        paper = "InventoryMoveItemEvent"
    )]
    fn inventory_stack_move(
        dimension: &str,
        source: [i32; 3],
        destination: [i32; 3],
        stack: &ItemStack,
        cancel: &mut bool,
    );
}
