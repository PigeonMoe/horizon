//! The `vehicle_client_move_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A server has validated a movement report from a boat or mount's
    /// driving player `player` (UUID). `vehicle` is the entity network ID.
    /// `from` and the proposed `to` are in `dimension`. Set `cancel` to keep
    /// the vehicle at `from`, clear its velocity, and send a correction to
    /// the driver. This hook does not cover server-driven movement such as
    /// minecart rail ticks, nor portal or command teleports.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no vehicle motion veto)",
        paper = "VehicleMoveEvent",
        cancellable = true
    )]
    fn vehicle_client_move_pre();
}
