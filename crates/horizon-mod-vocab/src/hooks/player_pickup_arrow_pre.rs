//! The `player_pickup_arrow_pre` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A server player is about to collect an arrow into their inventory. The
    /// arrow has stopped shaking, allows normal pickup, and at least part of
    /// its nonempty pickup stack fits. `position` is the arrow's position in
    /// `dimension`, and `player` is the collector's UUID. During the callback
    /// the arrow is being visited and cannot be edited through an entity
    /// handle. Set `cancel` to keep the arrow in the world and leave the
    /// inventory untouched. Creative-only pickups with no item do not fire
    /// this hook. `player_pickup_arrow` continues to observe successful pickups.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no such event)",
        paper = "PlayerPickupArrowEvent",
        cancellable = true
    )]
    fn player_pickup_arrow_pre();
}
