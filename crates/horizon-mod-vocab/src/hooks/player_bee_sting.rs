//! The damage left by an admitted bee sting on a server player.

pub trait Hooks {
    /// After an actual bee sting of the player has cleared the normal incoming protection,
    /// invulnerability and cooldown admission, before ordinary armor or health damage.
    /// The target's authoritative inventory is installed. Suppressing the damage here keeps
    /// the sting admitted, so its normal continuation can still leave a stinger and poison.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "suppress_damage",
        cancellable = true,
        target = "horizon_client::game::native_hooks::player_bee_sting::Game<'_>::native_player_bee_sting",
        paper = "EntityDamageByEntityEvent.setDamage(0) for an accepted Bee sting"
    )]
    fn player_bee_sting(player: PlayerRef, bee: EntityRef, damage: f32, suppress_damage: &mut bool);
}
