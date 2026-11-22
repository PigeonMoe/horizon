//! Notification of accepted server-side player movement and teleportation.

pub trait Hooks {
    /// The server has applied a client's movement or a player teleport.
    /// `from` and `to` are feet positions in their respective dimensions,
    /// or vehicle positions for accepted driving packets. Rotation-only
    /// movement, seating changes, join placement, death respawns and rejected
    /// moves produce no notification. The player is available in the new level.
    #[hook(side = "server", contract = "listen")]
    fn player_position_committed();
}
