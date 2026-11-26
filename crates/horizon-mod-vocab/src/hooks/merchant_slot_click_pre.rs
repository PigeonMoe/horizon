//! The `merchant_slot_click_pre` hook declaration.

pub trait Hooks {
    /// A server player clicked a slot in an open villager or wandering trader menu.
    /// If `merchant_slot` is true, `item` is the cursor stack. Otherwise it is the
    /// stack in the clicked player inventory slot. `hotbar_swap` identifies the
    /// numbered hotbar and offhand swap action. The player's state is swapped in.
    /// Cancelling preserves the inventory and resynchronizes the client. A drag
    /// or click outside the menu does not fire this hook.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no merchant inventory click hook)",
        paper = "InventoryClickEvent",
        cancellable = true
    )]
    fn merchant_slot_click_pre();
}
