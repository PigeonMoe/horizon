//! A read-only check on a player's crafting or workstation result.

pub trait Hooks {
    /// Checks a proposed result in a player's vanilla crafting or smithing
    /// menu. `menu` is `minecraft:crafting` for the inventory grid or
    /// crafting table, and `minecraft:smithing` for a smithing table.
    /// `inputs` contains the grid's four or nine cells in row order, or the
    /// smithing template, base and addition. Set `cancel` to leave the result
    /// empty and the ingredients available. This query also runs during
    /// result-slot takes and repeated crafts, so listeners must be read-only.
    /// The player UUID can be absent in a local menu without a profile.
    ///
    /// `minecraft:grindstone` and `minecraft:cartography_table` are checked
    /// only when taking the displayed result. Their inputs are the upper and
    /// lower slots. A refusal leaves the result visible and skips all movement,
    /// consumption, experience and map creation from the refused take.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        isolate = true,
        neoforge = "none (no read-only workstation result veto)",
        paper = "PrepareItemCraftEvent, PrepareSmithingEvent, InventoryClickEvent"
    )]
    fn player_recipe_result(
        player: Option<u128>,
        menu: &str,
        inputs: &[Option<ItemStack>],
        result: &ItemStack,
        cancel: &mut bool,
    );
}
