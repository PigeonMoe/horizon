//! Policy for a proposed redstone current change.

pub trait Hooks {
    /// The current at this block is about to change from `old` to `new`.
    /// Canceling keeps the old current without applying the associated change.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "BlockRedstoneEvent (restore old current)",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::block_redstone_current_pre::GameLevel<'_>::native_block_redstone_current_pre"
    )]
    fn block_redstone_current_pre();
}
