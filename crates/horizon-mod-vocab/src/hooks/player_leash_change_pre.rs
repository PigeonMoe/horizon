//! Declarations for the player leash change veto.

pub trait Hooks {
    /// A player is about to attach, transfer, or remove the lead on `entity`.
    /// `position` is the leashed entity's location in `dimension`. `attach`
    /// is true for a new holder and false for taking off the lead. Each entity
    /// in a batch can be vetoed independently. Natural snaps, death, and
    /// internal cleanup do not dispatch this hook.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none",
        paper = "PlayerLeashEntityEvent, PlayerUnleashEntityEvent",
        cancellable = true
    )]
    fn player_leash_change_pre();
}
