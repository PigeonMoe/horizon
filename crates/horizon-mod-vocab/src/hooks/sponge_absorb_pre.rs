//! The per-block sponge protection hook.

pub trait Hooks {
    /// A sponge at `sponge` in `dimension` is about to drain water at
    /// `pos`. Canceling retains that block while leaving it in the traversal,
    /// so the same sponge can still drain another candidate beyond it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "SpongeAbsorbEvent",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::sponge_absorb_pre::GameLevel<'_>::native_sponge_absorb_pre"
    )]
    fn sponge_absorb_pre();
}
