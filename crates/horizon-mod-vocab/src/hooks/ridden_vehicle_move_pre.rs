//! The `ridden_vehicle_move_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A vehicle carrying `player` (the passenger's UUID) is about to move.
    /// `vehicle` is its network entity ID and the endpoints are in `dimension`.
    /// Each player passenger, including nested passengers, can veto the same
    /// movement. Covers server physics as well as validated driver packets.
    /// A veto restores the source position and stops its movement. Teleports
    /// use their own hooks.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no vehicle motion veto)",
        paper = "VehicleMoveEvent",
        cancellable = true
    )]
    fn ridden_vehicle_move_pre();
}
