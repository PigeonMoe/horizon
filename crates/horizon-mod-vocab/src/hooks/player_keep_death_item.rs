//! The `player_keep_death_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Keep the player's `stack` in its original inventory `slot` through death and respawn.
    /// Slots are 0..36 for normal inventory, 36..40 for armor from feet to head, and 40 for the
    /// off hand. Runs before the stacks are taken, only when keep-inventory is off, and never for
    /// a spectator or a stack vanilla destroys with Curse of Vanishing. The kept stack is absent
    /// from `living_drops`. This hook reads the owner's state and does not copy the stack.
    #[hook(side = "server", host = "read", contract = "cancel", out = "keep")]
    fn player_keep_death_item(player: u128, slot: u32, stack: &ItemStack, keep: &mut bool);
}
