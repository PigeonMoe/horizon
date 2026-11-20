//! The `guest_boat_tick_move_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A guest-driven boat moved since its last server tick. This checks the
    /// final pose once per tick, after all accepted driver packets. Each real
    /// rider is checked in passenger-tree order until one vetoes.
    /// A veto stops the boat and offers a checked return to the previous
    /// pose. The failed hull return ejects its direct passengers and checks
    /// whether the vetoed rider can return there.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        paper = "VehicleMoveEvent",
        neoforge = "none (NeoForge has no vehicle motion veto)",
        cancellable = true
    )]
    fn guest_boat_tick_move_pre(
        player: u128,
        vehicle: u32,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
        cancel: &mut bool,
    );
}
