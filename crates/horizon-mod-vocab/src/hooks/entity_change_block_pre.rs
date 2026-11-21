//! Declarations carried from upstream `entity_change_block_v2`; the generator owns the typed bindings.

pub trait Hooks {
    /// An entity is about to change the block at `pos`, for example by
    /// trampling farmland or stripping copper patina with lightning. `player`
    /// is the responsible player for player actions. `entity` and `kind`
    /// identify the non-player entity for actions such as a lightning strike.
    /// `from` and `to` are resolved block states. Canceling prevents the block
    /// write and its neighbour updates, as with Bukkit's `EntityChangeBlockEvent`
    /// (@API:entity_change_block_v2@).
    /// The fork names this pre-change veto `entity_change_block_pre`; `entity_change_block` is the separate post-change notice.
    #[hook(
        side = "server",
        contract = "cancel",
        neoforge = "none (NeoForge has no such event; Bukkit's EntityChangeBlockEvent is cancellable)",
        cancellable = true,
        out = "cancel",
        target = "horizon_client::game::native_hooks::entity_change_block_pre::Game<'_>::native_entity_change_block_pre"
    )]
    fn entity_change_block_pre(
        player: Option<u128>,
        entity: Option<u32>,
        kind: &str,
        pos: [i32; 3],
        from: BlockState,
        to: BlockState,
        cancel: &mut bool,
    );
}
