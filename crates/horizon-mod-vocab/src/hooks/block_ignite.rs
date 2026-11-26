//! The `block_ignite` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0178

pub trait Hooks {
    /// On an authoritative server, fire is about to be placed or a block is
    /// about to be lit at `pos`. `player` is the acting server player, the
    /// direct source player for an explosion, or the resolved player owner
    /// of an arrow lighting a candle, candle cake or campfire. Set `cancel`
    /// to leave the position unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's BlockIgniteEvent)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_ignite"
    )]
    fn block_ignite();
}
