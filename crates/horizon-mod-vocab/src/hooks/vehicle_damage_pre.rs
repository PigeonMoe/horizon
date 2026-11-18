//! Target and attacker data for vehicle damage, including damage without an entity source.

pub trait Hooks {
    /// A boat or minecart (`vehicle`, network ID, and `kind`, namespaced type) is about to
    /// take damage in `dimension` at `position`. `player` names the resolved responsible
    /// player, if any. Canceling leaves the vehicle's hurt direction, damage and contents
    /// untouched. The separate `vehicle_damage` hook can still edit an admitted hit.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none",
        paper = "VehicleDamageEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::vehicle_damage_pre::Game<'_>::native_vehicle_damage_pre"
    )]
    fn vehicle_damage_pre();
}
