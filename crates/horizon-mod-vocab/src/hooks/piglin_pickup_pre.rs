//! A piglin is ready to take a dropped item.

pub trait Hooks {
    /// Called after a piglin finds a wanted item within pickup reach, before
    /// moving any of its stack into the piglin's hand or inventory. Cancelling
    /// keeps the item on the ground with its original quantity and movement.
    #[hook(
        side = "server",
        host = "read",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        neoforge = "none (piglin item pickup gate)",
        paper = "EntityPickupItemEvent"
    )]
    fn piglin_pickup_pre();
}
