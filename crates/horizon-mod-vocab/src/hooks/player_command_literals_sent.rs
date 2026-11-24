//! Per-player filtering of literal branches in server command suggestions.

pub trait Hooks {
    /// Hide literal branches from this player's command tree and completions.
    /// Each entry in `hidden_literals` is a space-separated path of literals
    /// from the root without a leading slash, for example `example admin`.
    /// Include each alias whose branch should be hidden. Command execution
    /// retains the registered permission checks.
    ///
    /// `player` is the player's UUID and `permission` is their current vanilla
    /// permission level. These values are available before a joining player
    /// has been placed in the world. As with `player_commands_sent`, later
    /// resends may reuse the last result for that player.
    #[hook(side = "server", contract = "collect", out = "hidden_literals")]
    fn player_command_literals_sent();
}
