//! The `block_spread` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0040

pub trait Hooks {
    /// On an authoritative server, `kind` identifies fluid flow (`fluid`),
    /// fire spread or burning (`fire`), or propagation by vegetation, buds
    /// or sculk (`growth`). `from` is the original source block.
    /// Set `canceled` to leave the block at `to` as it is. Fluids ask on
    /// every flow: answer fast.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_block_spread",
        neoforge = "none (BlockEvent.FluidPlaceBlockEvent covers only fluids making stone and the like, not flow or fire spread)"
    )]
    fn block_spread();
}
