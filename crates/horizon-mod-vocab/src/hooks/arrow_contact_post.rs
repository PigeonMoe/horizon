//! An accepted contact of a normal arrow on a logical server.

pub trait Hooks {
    /// A normal arrow `projectile`, shot by `owner` (0 when unresolved), has
    /// made an actual contact in `dimension`. `contact` is `damage` for an
    /// accepted entity hit, `hanging_break` for a removed item frame or
    /// painting, or `portal` after the level detects swept contact with a
    /// portal and before it registers a transition. `at` is the arrow's
    /// position in the source level. The arrow may already be gone after
    /// entity damage. A denied or deflected hit gives no observation.
    /// A block impact still uses `projectile_impact`.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (accepted arrow damage and swept portal contact)",
        paper = "EntityDamageByEntityEvent, HangingBreakByEntityEvent, EntityPortalEnterEvent"
    )]
    fn arrow_contact_post(
        dimension: &str,
        projectile: u32,
        owner: u32,
        at: [f64; 3],
        contact: &str,
    );
}
