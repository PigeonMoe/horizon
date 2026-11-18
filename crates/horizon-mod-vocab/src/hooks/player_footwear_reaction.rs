//! Footwear reactions at a player's physical action, before its outcome.

pub trait Hooks {
    /// Server-side footwear policy for `farmland_trample`, `fall_damage` or
    /// `ender_pearl_damage`. The player UUID, game mode, worn feet stack and optional
    /// `PlayerPersisted` compound belong to the player taking this exact action.
    /// For farmland a cancellation keeps the original block. For damage it prevents
    /// the hit and its side effects. The successful pearl teleport still takes place.
    /// A sound supplied with a canceled fall is played as a players-category level sound.
    /// This hook is answered from a read-only loop context, with no player mutation or
    /// combat reentry. It runs only for a nonempty worn feet slot on the server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "none (read-only action-scoped player equipment)",
        paper = "PlayerInteractEvent (PHYSICAL), EntityDamageEvent, EntityDamageByEntityEvent",
        cancellable = true,
        target = "horizon_client_lib::landing::native_player_footwear_reaction"
    )]
    fn player_footwear_reaction();
}
