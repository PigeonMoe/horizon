//! A player's click in an open brewing stand before any stacks move.

pub trait Hooks {
    /// Checks a click on a slot of an actual brewing stand menu, including
    /// its player inventory. `player_slot` distinguishes the player's slots
    /// from the five stand slots. `hotbar_swap` reports an available number-key
    /// or offhand swap. `clicked` and `carried` reflect the authoritative
    /// stacks before the click. Outside clicks and multi-slot drags are
    /// separate actions. Set `cancel` to leave all stacks untouched. A
    /// cancelled click can send `notice` to the clicking player.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        neoforge = "none (read-only brewing inventory click veto)",
        paper = "InventoryClickEvent"
    )]
    fn brewing_stand_click(
        player: Option<u128>,
        slot: usize,
        player_slot: bool,
        hotbar_swap: bool,
        clicked: Option<&ItemStack>,
        carried: Option<&ItemStack>,
        notice: &mut Option<Text>,
        cancel: &mut bool,
    );
}
