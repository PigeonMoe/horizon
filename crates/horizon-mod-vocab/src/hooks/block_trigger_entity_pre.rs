//! The entity admission policy for physical redstone triggers.

pub trait Hooks {
    /// An entity at `pos` is a candidate to activate a pressure plate, an
    /// attached tripwire, or a wooden button. Each relevant candidate is
    /// checked before contributing: a weighted plate counts all admitted
    /// entities, a binary plate and an attached tripwire accept the first
    /// admitted entity, and a button checks its first arrow. The source player
    /// fields identify the player physically in the block, if any.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (entity admission to a block's signal query)",
        paper = "PlayerInteractEvent (PHYSICAL) and EntityInteractEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::block_trigger_entity_pre::GameLevel<'_>::native_block_trigger_entity_pre"
    )]
    fn block_trigger_entity_pre(
        dimension: LevelDimension,
        pos: BlockPos,
        entity: u32,
        player: Option<u128>,
        name: &str,
        permission_level: u8,
        cancel: &mut bool,
    );
}
