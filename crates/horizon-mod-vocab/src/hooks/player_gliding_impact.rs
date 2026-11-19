//! Protective headwear at the actual damage boundary of an elytra impact.

pub trait Hooks {
    /// Server impact while gliding or on the tick the glide ends. `action` is either
    /// `fall_damage` or `fly_into_wall`. The returned response can cancel the hit,
    /// play a registered players-category sound to its wearer, and request one point
    /// of headwear damage. The wear uses the borrowed head slot, including creative,
    /// Unbreakable and armor Unbreaking, and only runs on cancellation.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "none (action-scoped equipment protection and wear)",
        paper = "EntityDamageEvent and EntityToggleGlideEvent",
        cancellable = true,
        target = "horizon_client_lib::landing::native_player_gliding_impact"
    )]
    fn player_gliding_impact();
}
