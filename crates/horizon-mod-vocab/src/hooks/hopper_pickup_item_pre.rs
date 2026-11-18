//! The `hopper_pickup_item_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A block hopper or hopper minecart at `pos` is about to take a dropped
    /// item at `source`, its exact position in `dimension`. Set `cancel` to
    /// keep the item in the world and the hopper's inventory intact. For a
    /// block hopper the older `hopper_pickup_item` hook runs first.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no hopper pickup event)",
        paper = "InventoryPickupItemEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::hopper_pickup_item_pre::GameLevel<'_>::native_hopper_pickup_item_pre"
    )]
    fn hopper_pickup_item_pre();
}
