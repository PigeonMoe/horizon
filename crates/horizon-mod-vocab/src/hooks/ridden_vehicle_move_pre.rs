//! The `ridden_vehicle_move_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A vehicle carrying `player` (the passenger's UUID) is about to move.
    /// `vehicle` is its network entity ID and the endpoints are in `dimension`.
    /// Each player passenger, including nested passengers, can veto the same
    /// movement. Covers server physics as well as validated driver packets.
    /// A veto restores the source position. Server-driven living mounts keep
    /// their velocity. Boats, minecarts and validated driver packets stop their
    /// movement. A veto can also set `paper_rollback` to request a checked
    /// same-level teleport for a server-driven boat or minecart. If that attempt is
    /// vetoed, its direct passengers are ejected and the denied player is
    /// offered a checked return to the source. Other movers ignore the request.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        version = 2,
        neoforge = "none (NeoForge has no vehicle motion veto)",
        paper = "VehicleMoveEvent",
        cancellable = true
    )]
    fn ridden_vehicle_move_pre(
        player: u128,
        vehicle: u32,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
        paper_rollback: &mut bool,
        cancel: &mut bool,
    );
}
