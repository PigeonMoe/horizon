//! Damage admission for an entity-sourced hit, with the authoritative player owner.

pub trait Hooks {
    /// An entity-sourced hit about to affect `target_kind` at `position`
    /// in `dimension`. `alive_kind` identifies a living-entity victim.
    /// `player` is the responsible player when they are connected (direct
    /// attacker, shooter, or owner), not necessarily the direct entity.
    /// A veto applies only to this victim and prevents damage.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "level",
        neoforge = "none",
        paper = "EntityDamageByEntityEvent",
        cancellable = true,
        target = "horizon_client::game::native_hooks::entity_damage_target_pre::HitLevel<'_>::native_entity_damage_target_pre"
    )]
    fn entity_damage_target_pre();
}
