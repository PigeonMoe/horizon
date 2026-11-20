//! The player's loaded client requested sprinting or walking.

pub trait Hooks {
    /// Before applying a loaded player's START_SPRINTING or STOP_SPRINTING.
    /// Every admitted packet is heard, including one repeating the current state.
    /// The real player's body and inventory are installed, with the sprint state
    /// as it was before the packet. Cancel to keep it. The older `player_sprint`
    /// hook still observes only actual changes after admission.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        paper = "PlayerToggleSprintEvent"
    )]
    fn player_sprint_pre(player: PlayerRef, sprinting: bool, cancel: &mut bool);
}
