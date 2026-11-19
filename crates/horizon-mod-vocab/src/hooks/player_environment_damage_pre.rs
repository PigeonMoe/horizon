//! A server player's admitted damage from the environment.

pub trait Hooks {
    /// After invulnerability, existing protection and the damage cooldown
    /// admit the hit, before it affects the player's armor or health.
    /// The amount is the pending hit before armor, magic and absorption.
    /// The player's authoritative items, ender chest and effects are installed.
    /// Currently emitted for direct contact with fire or soul fire, both
    /// using the vanilla `minecraft:in_fire` damage source. Burning after
    /// leaving the fire uses `minecraft:on_fire` and goes on its usual path.
    /// Cancelling this hit preserves the fire and ignition of the contact.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        paper = "EntityDamageEvent (FIRE)"
    )]
    fn player_environment_damage_pre(
        player: PlayerRef,
        damage_type: &str,
        damage: f32,
        cancel: &mut bool,
    );
}
