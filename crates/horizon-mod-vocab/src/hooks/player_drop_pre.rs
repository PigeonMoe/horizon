//! The `player_drop_pre` hook declarations.

pub trait Hooks {
    /// A server player is about to drop a stack using the hotbar drop key or a
    /// manual drop in their inventory or an ordinary storage menu. `player` is their UUID and
    /// `position` is where the item would spawn in `dimension`. Set `cancel` to
    /// leave the item in its slot or on the cursor and correct the client's
    /// prediction. This runs before any removal or entity spawn. Death drops,
    /// spillover, station menus, native menu callbacks and creative drop packets are separate.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (ItemTossEvent happens after removal)",
        paper = "PlayerDropItemEvent",
        cancellable = true
    )]
    fn player_drop_pre();
}
