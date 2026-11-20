//! A server player's admitted damage from the environment.

pub trait Hooks {
    /// After invulnerability, existing protection and the damage cooldown
    /// admit the hit, before it affects the player's armor or health.
    /// The amount is the pending hit before armor, magic and absorption.
    /// The player's authoritative items, ender chest and effects are installed.
    /// Emitted for direct contact with fire or soul fire (`minecraft:in_fire`),
    /// and for unattributed `minecraft:fall` damage after equipment has had a
    /// chance to cancel the hit. Cancelling fire contact preserves its fire and
    /// ignition. Cancelling fall damage preserves the landing itself.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        paper = "EntityDamageEvent (FIRE, FALL)"
    )]
    fn player_environment_damage_pre(
        player: PlayerRef,
        damage_type: &str,
        damage: f32,
        cancel: &mut bool,
    );
}
