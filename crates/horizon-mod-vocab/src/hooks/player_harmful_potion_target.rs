//! The `player_harmful_potion_target` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// A player-owned splash potion or lingering cloud with at least one
    /// harmful effect is about to dose `target` (network ID), of `target_kind`
    /// at `position` in `dimension`. `player` is the thrower's UUID. Canceling
    /// leaves this target's effects, cloud reapplication timer, radius use
    /// and duration use untouched. Other targets are still considered.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (NeoForge has no equivalent splash and cloud target veto)",
        paper = "PotionSplashEvent, AreaEffectCloudApplyEvent",
        cancellable = true
    )]
    fn player_harmful_potion_target();
}
