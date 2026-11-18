//! Protection before a falling block changes its starting or landing block.

pub trait Hooks {
    /// A falling block will change `pos` in `dimension`. `origin_dimension` is
    /// the original level ID, or empty when the entity's origin is unknown.
    /// `origin_pos` is meaningful only when that level ID is present.
    /// Canceling leaves the world as it was and prevents a landing drop.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "EntityChangeBlockEvent (FallingBlock)",
        neoforge = "none (NeoForge has no such event)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::falling_block_change_pre::GameLevel<'_>::native_falling_block_change_pre"
    )]
    fn falling_block_change_pre();
}
