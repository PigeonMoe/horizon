//! Per-block notice after a mob's direct break has been allowed.

pub trait Hooks {
    /// The mob `entity` of type `kind` is about to break `state` at `pos`.
    /// The griefing and block-change checks and the living destruction veto
    /// have admitted the break. The block and its live block entity are still
    /// readable. A part may replace the block through the server host. If the
    /// state changes, the game leaves the replacement in place and skips the
    /// original break's particles, loot and block entity removal. This notice
    /// covers a mob's direct break, including the Wither's block sweep.
    #[hook(side = "server", contract = "listen", scope = "block", key = "state")]
    fn on_mob_block_break();
}
