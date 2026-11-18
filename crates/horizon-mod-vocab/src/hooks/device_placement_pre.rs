//! The `device_placement_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A device asks whether `owner` can build at `pos` in `dimension`.
    /// The UUID is the saved owner, who need not be online. `operator` comes
    /// from the operator list or the available owner's server permissions.
    /// Cancel to deny the device. The permission check itself changes no blocks.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client::game::native_hooks::device_placement_pre::Game<'_>::native_device_placement_pre"
    )]
    fn device_placement_pre(
        owner: u128,
        operator: bool,
        dimension: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    );
}
