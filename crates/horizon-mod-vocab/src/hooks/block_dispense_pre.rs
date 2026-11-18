//! The `block_dispense_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A dispenser or ejecting dropper at `pos` selected `item` and is about
    /// to act toward `target`. Both block positions are in `dimension`. The
    /// slot choice has already drawn its random number. Canceling leaves the
    /// item in its slot and skips the dispense behavior and effects. Dropper
    /// transfers into an inventory use `inventory_move_item` instead.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge controls individual dispense behaviors)",
        paper = "BlockDispenseEvent",
        cancellable = true
    )]
    fn block_dispense_pre();
}
