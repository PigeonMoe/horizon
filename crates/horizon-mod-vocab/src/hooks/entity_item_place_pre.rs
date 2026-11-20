//! Veto a player's item placing an entity at its final position.

pub trait Hooks {
    /// A player's item is about to place a boat, minecart, armor stand, or end
    /// crystal of type `kind` in `dimension`. `position` is the entity's final
    /// position, including any applied stack data. Canceling leaves the item
    /// in hand and prevents insertion and the placement game event. Hanging
    /// entities and dispensers use their own placement hooks.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (the NeoForge EntityPlaceEvent places a block)",
        paper = "EntityPlaceEvent",
        cancellable = true
    )]
    fn entity_item_place_pre(
        dimension: &str,
        kind: &str,
        position: [f64; 3],
        player: u128,
        cancel: &mut bool,
    );
}
