//! Adjust the committed proposal shown by an enchanting table.

pub trait Hooks {
    /// An admitted enchanting-table button press. Change the actual
    /// (enchantment id, level) list before the player pays levels or lapis.
    /// Only registered enchantments at levels 1 to 255 are applied.
    /// An empty list leaves the item and costs untouched. For a plain book,
    /// `book_enchantments` goes on its regular enchantments component before
    /// conversion, while `enchantments` goes into the book's stored component.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (mutable server table proposal)",
        paper = "EnchantItemEvent"
    )]
    fn player_enchant_item_pre(
        player: PlayerRef,
        input: &ItemStack,
        enchantments: &mut Vec<(String, u32)>,
        book_enchantments: &mut Vec<(String, u32)>,
    );
}
