//! Read-only admission while mob AI or a vehicle proposes a block change.

pub trait Hooks {
    /// Before a mob or vehicle (`entity`, network ID; `kind`, namespaced type)
    /// changes the block at `pos` in `dimension`. Canceling prevents the associated
    /// action, such as taking a block into an enderman's hand, merging a
    /// silverfish into stone, or breaking a lily pad under a boat.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none",
        paper = "EntityChangeBlockEvent (mob AI and lily pad collision)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::mob_block_change_pre::Game<'_>::native_mob_block_change_pre"
    )]
    fn mob_block_change_pre();
}
