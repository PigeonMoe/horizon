//! The `entity_teleport_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// Veto a same-level entity teleport. Currently covers server-driven boat
    /// and minecart rollbacks after a denied rider's move
    /// (`cause = "vehicle_rollback"`). The network
    /// entity ID may be held by the tick. The supplied positions and yaw/pitch
    /// describe the attempted move and the proposed return, in that order.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        paper = "EntityTeleportEvent",
        neoforge = "none (NeoForge has separate teleport events)",
        cancellable = true
    )]
    fn entity_teleport_pre(
        entity: u32,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
        from_rotation: [f32; 2],
        to_rotation: [f32; 2],
        cause: &str,
        cancel: &mut bool,
    );
}
