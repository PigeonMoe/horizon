//! The `player_attack_at_pre` hook: declarations the mod API generator reads.
use crate::PlayerRef;

pub trait Hooks {
    /// A player asks whether a PvP ability can target the loaded block at `pos` in
    /// `dimension`. PvP is enabled in the level. Cancel to deny the location. This
    /// query does not imply a victim or replace the hooks for actual entity damage.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client::game::native_hooks::player_attack_at_pre::Game<'_>::native_player_attack_at_pre"
    )]
    fn player_attack_at_pre(player: PlayerRef, dimension: &str, pos: [i32; 3], cancel: &mut bool);
}
