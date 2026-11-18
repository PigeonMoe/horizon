//! Policy over individual writes staged by live growth or bone meal.

pub trait Hooks {
    /// `pos` is a pending write from growth anchored at `origin`. During the
    /// capture, generation reads all proposed writes. Canceling here prevents
    /// only this committed write. `structure` marks tree and fungus features,
    /// and `fertilized` marks a bone meal application. A player use includes
    /// its `player`, `name` and `permission_level`; dispensers have no player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        paper = "StructureGrowEvent and BlockFertilizeEvent (candidate writes)",
        neoforge = "none (captured growth block states)",
        cancellable = true,
        target = "horizon_client::game::native_hooks::block_growth_candidate_pre::GameLevel<'_>::native_block_growth_candidate_pre"
    )]
    fn block_growth_candidate_pre(
        dimension: LevelDimension,
        origin: BlockPos,
        pos: BlockPos,
        structure: bool,
        fertilized: bool,
        player: Option<u128>,
        name: &str,
        permission_level: u8,
        cancel: &mut bool,
    );
}
