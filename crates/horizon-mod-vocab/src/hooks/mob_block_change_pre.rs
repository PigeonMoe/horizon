//! Read-only admission for block changes caused by mobs, vehicles and projectiles.

pub trait Hooks {
    /// Before an entity (`entity`, network ID; `kind`, namespaced type)
    /// changes the block at `pos` in `dimension`. Canceling prevents the associated
    /// action, such as taking a block into an enderman's hand, merging a
    /// silverfish into stone, or breaking a block with a boat or projectile.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none",
        paper = "EntityChangeBlockEvent (mob AI, boats and projectile impacts)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::mob_block_change_pre::Game<'_>::native_mob_block_change_pre"
    )]
    fn mob_block_change_pre();
}
