//! The `entity_teleport_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// Veto or redirect an entity teleport. Covers boat and minecart
    /// rollbacks after a denied rider's move (`cause = "vehicle_rollback"`).
    /// The network entity ID may be held by the tick. `from` and `from_rotation`
    /// are the attempted pose in `dimension`. Change `to_dimension`, `to` and
    /// `to_rotation` to select a destination. A cross-level vehicle rollback
    /// keeps the riders whose individual teleports succeed. Cancellation or an
    /// invalid destination ejects the passengers. Only the rider
    /// whose move was denied is offered a separate teleport to the original
    /// return point.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        paper = "EntityTeleportEvent",
        neoforge = "none (NeoForge has separate teleport events)",
        cancellable = true,
        version = 3
    )]
    fn entity_teleport_pre(
        entity: u32,
        dimension: &str,
        from: [f64; 3],
        to_dimension: &mut Identifier,
        to: &mut [f64; 3],
        from_rotation: [f32; 2],
        to_rotation: &mut [f32; 2],
        cause: &str,
        cancel: &mut bool,
    );
}
