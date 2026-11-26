//! The full stack a player has just broken.

pub trait Hooks {
    /// A server player's single item was actually broken by durability loss.
    /// `stack` is the worn stack before the count was decremented, including
    /// its components. `inventory_slot` is the cell that was changed, using
    /// `PlayerRef::inventory_slots` numbering. It stays the same if the
    /// player changes selection before a listener's scheduled task runs.
    /// The break has already committed. This event observes it without
    /// replacing or canceling the old `player_destroy_item` event.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "PlayerDestroyItemEvent",
        paper = "PlayerItemBreakEvent"
    )]
    fn player_item_break_snapshot(player: u128, inventory_slot: u32, stack: &ItemStack);
}
