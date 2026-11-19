//! Lets a server mod suppress a rocket's damage without changing the burst.
// catalog-order: 0353

pub trait Hooks {
    /// A rocket's burst is about to damage entities. The exploding rocket may
    /// be out of the entity list for its tick, so `item` borrows a snapshot
    /// of its original carried stack, including components loaded from disk.
    /// Cancelling here skips damage to every target. The burst packet, sounds,
    /// particles, vibration and `firework_explode` still occur normally.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (firework explosion damage gate)",
        cancellable = true
    )]
    fn firework_damage_pre(entity: u32, pos: [f64; 3], item: &ItemStack, cancel: &mut bool);
}
