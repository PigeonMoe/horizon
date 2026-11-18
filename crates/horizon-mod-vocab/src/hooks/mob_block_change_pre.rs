//! Read-only admission while mob AI proposes a block change.

pub trait Hooks {
    /// Before a mob (`entity`, network ID; `kind`, namespaced type) changes the block at
    /// `pos` in `dimension`. Canceling prevents the AI's associated action, such as taking
    /// a block into an enderman's hand or merging a silverfish into stone.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none",
        paper = "EntityChangeBlockEvent (mob AI)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::mob_block_change_pre::Game<'_>::native_mob_block_change_pre"
    )]
    fn mob_block_change_pre();
}
