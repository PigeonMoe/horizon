//! The `player_teleport_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A server player's exact landing point is ready for a spectator, ender
    /// pearl, or random teleport. `player` is the UUID. `from` and `to` are
    /// feet coordinates in `from_level` and `to_level`. The causes are
    /// `spectate`, `ender_pearl`, and `random`, which includes chorus fruit
    /// after border, ground, and collision checks. Set `cancel` to keep the
    /// player at the source. A vetoed pearl also skips its arrival effects.
    /// Command teleports use `teleport_command`; this hook does not cover
    /// portals, respawns, login placement, or corrections sent to clients.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has separate teleport events)",
        paper = "PlayerTeleportEvent",
        cancellable = true
    )]
    fn player_teleport_pre(
        player: u128,
        from_level: &str,
        from: [f64; 3],
        to_level: &str,
        to: [f64; 3],
        cause: &str,
        cancel: &mut bool,
    );
}
