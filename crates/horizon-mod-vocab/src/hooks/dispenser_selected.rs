//! The `dispenser_selected` hook: declarations the mod API generator reads.

pub trait Hooks {
    /// An authoritative dispenser has selected a nonempty `slot` and its complete `stack`.
    /// `pos` and the block it faces (`target`) are in `dimension`. `viewed` means a player
    /// in this dimension has this stock inventory open, including spectators. This runs before the
    /// ordinary `block_dispense_pre` policy. Cancel to replace the dispenser's behavior
    /// without consuming the item or playing its vanilla effects. Droppers do not call it.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        target = "horizon_client::game::native_hooks::block_dispense_pre::Game<'_>::native_dispenser_selected"
    )]
    fn dispenser_selected(
        dimension: &str,
        pos: [i32; 3],
        target: [i32; 3],
        slot: u32,
        stack: &ItemStack,
        viewed: bool,
        cancel: &mut bool,
    );
}
