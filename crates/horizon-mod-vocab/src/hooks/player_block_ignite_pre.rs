//! Player attribution for a block ignition.

pub trait Hooks {
    /// Checks a player-attributed block ignition after `block_ignite` allowed
    /// it. Carries the live player name and permission level for access checks.
    /// Lighting a candle, candle cake or campfire with an arrow uses the
    /// arrow's resolved player owner. An explosion uses only its direct
    /// source player. Cancel to prevent the fire or lit state from being written.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        isolate = true,
        neoforge = "none (player-attributed block ignition)",
        paper = "BlockIgniteEvent",
        target = "horizon_client::game::native_hooks::player_block_ignite_pre::GameLevel<'_>::native_player_block_ignite_pre"
    )]
    fn player_block_ignite_pre(
        dimension: LevelDimension,
        pos: BlockPos,
        cause: &str,
        player: u128,
        name: &str,
        permission_level: u8,
        cancel: &mut bool,
    );
}
