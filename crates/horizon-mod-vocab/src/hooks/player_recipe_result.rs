//! A read-only check on a player's crafting or smithing result.

pub trait Hooks {
    /// Checks a proposed result in a player's vanilla crafting or smithing
    /// menu. `menu` is `minecraft:crafting` for the inventory grid or
    /// crafting table, and `minecraft:smithing` for a smithing table.
    /// `inputs` contains the grid's four or nine cells in row order, or the
    /// smithing template, base and addition. Set `cancel` to leave the result
    /// empty and the ingredients available. This query also runs during
    /// result-slot takes and repeated crafts, so listeners must be read-only.
    /// The player UUID can be absent in a local menu without a profile.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        host = "read",
        isolate = true,
        neoforge = "none (no read-only crafting or smithing result veto)",
        paper = "PrepareItemCraftEvent, PrepareSmithingEvent"
    )]
    fn player_recipe_result(
        player: Option<u128>,
        menu: &str,
        inputs: &[Option<ItemStack>],
        result: &ItemStack,
        cancel: &mut bool,
    );
}
