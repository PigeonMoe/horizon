//! Equipment-induced reactions to a server player's fall.

pub trait Hooks {
    /// After ordinary incoming damage, protection and damage cooldown allow fall damage, while the
    /// falling player's real feet slot and saved research are borrowed. A listener
    /// that cancels the fall supplies a nonzero, mod-defined identifier as the
    /// first member of `effect`, delivered once to `player_fall_reaction_post`.
    /// The optional `(sound, pitch)` in the second member plays
    /// to the falling player at their eyes, with volume 1 and the supplied pitch.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        cancellable = true,
        target = "horizon_client_lib::landing::native_player_fall_reaction",
        paper = "EntityDamageEvent (FALL), delayed level writes"
    )]
    fn player_fall_reaction();

    /// Apply the side effects selected by a canceled fall, after releasing the
    /// movement borrow. The actor's authoritative body is still installed and the
    /// location and bounds come from the fall itself.
    #[hook(
        side = "server",
        contract = "listen",
        target = "horizon_client::game::native_hooks::player_fall_reaction_post::Game<'_>::native_player_fall_reaction_post"
    )]
    fn player_fall_reaction_post(
        player: PlayerRef,
        reaction: u128,
        origin: [f64; 3],
        bounds: [[f64; 3]; 2],
        damage: f32,
    );
}
