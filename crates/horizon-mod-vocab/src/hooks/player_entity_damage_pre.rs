//! A server player's admitted damage from a direct entity.

pub trait Hooks {
    /// After the protection, invulnerability and hit cooldown checks admit an entity hit,
    /// before it affects armor or health. The player's authoritative inventory and effects
    /// are installed. The source names the direct entity even when the game is updating its
    /// projectile outside the level's queryable entity list. `projectile` identifies that
    /// source as a projectile. This also covers the Paper projectile damager for a borrowed
    /// ownerless wither skull or a pearl's self-damage with no vanilla direct entity.
    /// Canceling keeps the player from taking the hit. See
    /// `ServerHost::return_player_damage_projectile` to also replace the projectile.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        paper = "EntityDamageByEntityEvent"
    )]
    fn player_entity_damage_pre(
        player: PlayerRef,
        source: &HurtSource<'_>,
        projectile: bool,
        damage: f32,
        cancel: &mut bool,
    );
}
