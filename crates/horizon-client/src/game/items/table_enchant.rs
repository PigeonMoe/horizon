//! The enchanting table's result (`EnchantmentMenu.clickMenuButton`), with
//! `ItemPart::apply_enchantments` (`IItemExtension.applyEnchantments`) asked for it.
use super::*;
use crate::game::TryIntoApi;
use horizon_items::item_data::Levels;

/// Says once per item that its `apply_enchantments` parts answered a count other than the
/// input's, which the table can't hold.
fn refuse_table_count(item: &str, input: u32, answered: u32) {
    static REFUSED: std::sync::Mutex<std::collections::BTreeSet<String>> =
        std::sync::Mutex::new(std::collections::BTreeSet::new());
    if REFUSED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(item.to_owned())
    {
        log::warn!(
            "Horizon: an apply_enchantments part of {item} answered a count of {answered} for an input of {input}; the table's own result is used"
        );
    }
}

impl Game<'_> {
    /// The stack the table's `row`, costing `cost`, makes of `input` at `seed`: vanilla's
    /// (`horizon_client::stations::apply_enchantment`), then the parts of the input's item
    /// (`ItemPart::apply_enchantments`) with the list it chose. `None` when the row gives
    /// nothing. A result whose count isn't the input's is refused for vanilla's. An item with
    /// no such part costs one table lookup.
    pub(in crate::game) fn table_enchant(
        &self,
        seed: i32,
        row: usize,
        cost: i32,
        input: &ItemStack,
    ) -> Option<ItemStack> {
        let list = self.table_selection(seed, row, cost, input);
        self.apply_table_selection(input, &list)
    }

    fn table_selection(&self, seed: i32, row: usize, cost: i32, input: &ItemStack) -> Levels {
        horizon_client::stations::enchantment_list(
            &self.stations().enchantments,
            self.items(),
            seed,
            row,
            cost,
            input,
        )
    }

    /// Resolve the final table proposal on the admitted player's server turn.
    pub(in crate::game) fn table_enchant_for_player(
        &mut self,
        seed: i32,
        row: usize,
        cost: i32,
        input: &ItemStack,
    ) -> Option<(ItemStack, Levels)> {
        let mut list = self.table_selection(seed, row, cost, input);
        if list.is_empty() {
            return None;
        }

        if mod_api::__private::listened(mod_api::HookId::PlayerEnchantItemPre)
            && let Some(player) = self.native_acting_player()
            && let Ok(api_input) = input.try_into_api()
        {
            let mut enchantments: Vec<_> = list
                .iter()
                .map(|(id, level)| (id.to_string(), *level))
                .collect();
            let mut book_enchantments = Vec::new();
            self.native_player_enchant_item_pre(mod_api::__private::PlayerEnchantItemPreCall {
                player,
                input: &api_input,
                enchantments: &mut enchantments,
                book_enchantments: &mut book_enchantments,
            });
            let registry = &self.stations().enchantments;
            let registered = |(id, level): &(String, u32)| {
                (1..=255).contains(level) && registry.get(id).is_some()
            };
            list = enchantments
                .into_iter()
                .filter(registered)
                .map(|(id, level)| (id.into(), level))
                .collect();
            if &*input.item == "minecraft:book" && !book_enchantments.is_empty() {
                let mut book = input.clone();
                for (id, level) in book_enchantments.into_iter().filter(registered) {
                    book.enchant(&id, level);
                }
                return Some((self.apply_table_selection(&book, &list)?, list));
            }
        }

        Some((self.apply_table_selection(input, &list)?, list))
    }

    fn apply_table_selection(&self, input: &ItemStack, list: &Levels) -> Option<ItemStack> {
        let vanilla = horizon_client::stations::apply_enchantment_list(input, list.clone())?;
        if !mod_api::__private::item_listens(mod_api::HookId::ApplyEnchantments, &input.item) {
            return Some(vanilla);
        }
        let mut result = vanilla.clone();
        self.native_apply_enchantments(input, list, &mut result);
        if result.count != input.count {
            refuse_table_count(&input.item, input.count, result.count);
            return Some(vanilla);
        }
        Some(result)
    }
}
