//! The `device_break_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A device asks whether its saved `owner` can break blocks at `pos` in `dimension`.
    /// The owner can be offline. `operator` comes from the operator list or the live player's
    /// server permissions. Cancel to deny permission. This query does not change a block.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client::game::native_hooks::device_break_pre::Game<'_>::native_device_break_pre"
    )]
    fn device_break_pre(
        owner: u128,
        operator: bool,
        dimension: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    );
}
