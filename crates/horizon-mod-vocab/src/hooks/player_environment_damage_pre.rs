//! A server player's admitted damage from the environment.

pub trait Hooks {
    /// After invulnerability, existing protection and the damage cooldown
    /// admit the hit, before it affects the player's armor or health.
    /// The amount is the pending hit before armor, magic and absorption.
    /// The player's authoritative items, ender chest and effects are installed.
    /// Emitted for direct fire contact (`minecraft:in_fire`), unattributed
    /// `minecraft:fall` after equipment protection, `minecraft:starve`,
    /// lava contact (`minecraft:lava`) and loss of air (`minecraft:drown`).
    /// Cancelling a hit preserves the contact, landing, hunger or air tick.
    /// Burning after leaving the fire uses `minecraft:on_fire` on its usual path.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        paper = "EntityDamageEvent (FIRE, FALL, STARVATION, LAVA, DROWNING)"
    )]
    fn player_environment_damage_pre(
        player: PlayerRef,
        damage_type: &str,
        damage: f32,
        cancel: &mut bool,
    );
}
