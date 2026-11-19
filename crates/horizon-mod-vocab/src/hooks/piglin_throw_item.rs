//! The piglin's outcome before it throws one item toward a nearby player.

pub trait Hooks {
    /// An item the piglin is about to throw after a barter, or a previous
    /// main-hand item it is throwing away. The stack carries the rolled
    /// count and components. Return a replacement stack to throw instead.
    /// An unchanged outcome keeps the ordinary off-hand swing and trajectory.
    #[hook(
        side = "server",
        host = "read",
        contract = "replace",
        out = "replacement",
        neoforge = "none (piglin item thrown before the entity is added)",
        paper = "EntityDropItemEvent"
    )]
    fn piglin_throw_item();
}
