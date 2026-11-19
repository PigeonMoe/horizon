//! Adjust the experience a player receives from the orb they just picked up.

pub trait Hooks {
    /// The points left from a single orb pickup after Mending repaired the
    /// collector's items and before those points enter their player experience.
    /// Only positive remainders reach this hook. An answer of zero or below
    /// gives no points. Direct awards and XP commands use their own paths.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (orb pickup remainder after Mending)",
        paper = "PlayerExpChangeEvent"
    )]
    fn player_orb_xp_pre(player: PlayerRef, points: &mut i32);
}
