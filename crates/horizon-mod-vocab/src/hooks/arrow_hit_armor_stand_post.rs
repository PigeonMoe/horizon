//! Admitted ordinary arrow damage to an armor stand.
use crate::ItemStack;

pub trait Hooks {
    /// A normal arrow hit an armor stand after the protection and native damage
    /// decisions allowed it. `position` is the stand's feet before the hit,
    /// `damage` is the arrow's raw impact amount, and `weapon` is the weapon
    /// recorded on the arrow. The stand can already be gone. The resolved
    /// `owner` is 0 when there is no longer a source entity. A veto gives no event.
    #[hook(
        side = "server",
        contract = "listen",
        paper = "EntityDamageByEntityEvent"
    )]
    fn arrow_hit_armor_stand_post(
        dimension: &str,
        projectile: u32,
        owner: u32,
        stand: u32,
        position: [f64; 3],
        damage: f32,
        weapon: Option<&ItemStack>,
    );
}
