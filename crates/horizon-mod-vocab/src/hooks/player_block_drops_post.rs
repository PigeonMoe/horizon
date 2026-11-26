//! The finalized player block drop hook.

pub trait Hooks {
    /// A server player has harvested a block at `pos` with old `state`. Its
    /// item drops survived `block_drops` and are about to be delivered. The
    /// player's authoritative inventory reflects the wear from this break.
    /// `drops` is the accepted read-only item entity list, which may be
    /// empty. The server host can issue messages or spawn independent bonus
    /// items. This hook does not run for a cancelled drop list, a creative
    /// break, or a level-side break with no player.
    #[hook(side = "server", contract = "listen", paper = "BlockDropItemEvent")]
    fn player_block_drops_post(
        player: PlayerRef,
        pos: [i32; 3],
        state: BlockState,
        drops: &[ItemDrop],
    );
}
