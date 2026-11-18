//! Policy for an individual physics recipient or a newly placed block's shape updates.

pub trait Hooks {
    /// Before the block at `pos` handles a queued neighbor callback (`shapes = false`),
    /// or sends the shape updates of its newly committed state (`shapes = true`).
    /// Canceling the physics leaves the committed block and other recipients unchanged.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "BlockPhysicsEvent",
        neoforge = "none",
        cancellable = true,
        target = "horizon_client::game::native_hooks::block_physics_pre::GameLevel<'_>::native_block_physics_pre"
    )]
    fn block_physics_pre();
}
