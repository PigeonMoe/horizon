//! The `player_first_join` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0352

pub trait Hooks {
    /// The server placed `player` (`name`) without loading any previous profile
    /// for that player. Heard after `player_logged_in`, with the new player's
    /// inventory available. An arrival that imported a profile does not count
    /// as a first join.
    #[hook(side = "server", contract = "listen")]
    fn player_first_join();
}
