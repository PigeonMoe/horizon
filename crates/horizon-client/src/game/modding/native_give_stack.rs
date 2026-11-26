//! Whole stacks for server mods: one given to a player as `/give` gives it
//! (`player_give_stack`), and saved item stack codecs.
use super::Game;
use horizon_client::{items::ItemStack, lan::playerdata};
use horizon_core::nbt::{self, Compound, Limits, Tag};
use mod_api::{PlayerDataRefusal, PlayerRef};
use std::io::{Read, Write};

/// The most SNBT text `item_from_snbt` reads: `NbtAccounter`'s default quota, 2 MiB.
const MAX_SNBT_BYTES: usize = 2 * 1024 * 1024;
const MAX_SAVED_GZIP_BYTES: usize = 1024 * 1024;
const MAX_SAVED_NBT_BYTES: usize = 16 * 1024 * 1024;

impl Game<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up, in the player's turn.
    pub(super) fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        let ran = self.native_as_player_handle(player, |game| {
            game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `ItemStack.CODEC` reading the compound `snbt` stands for (`TagParser`): only that
    /// compound and whitespace. At most 2 MiB and 512 levels: vanilla's SNBT parser has
    /// neither limit (deep text ends in a stack overflow); they are this API's bound so
    /// mod-supplied text cannot overflow the parser.
    pub(super) fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack> {
        if snbt.len() > MAX_SNBT_BYTES {
            return None;
        }
        let (tag, end) = horizon_core::nbt::parse_snbt(snbt).ok()?;
        if !snbt.as_bytes()[end..].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        // `ItemStack.CODEC`'s `count` is 1 to 99 and is refused outside it: a stack read here
        // is never a different count from the one written.
        if let Some(count) = tag.get("count")
            && horizon_client_core::nbt_value::number(Some(count))
                .is_none_or(|n| !(1..=99).contains(&n))
        {
            return None;
        }
        let stack = horizon_items::item_nbt::item_from_nbt(&tag)?;
        self.items().get(&stack.item).is_some().then_some(stack)
    }

    pub(super) fn item_from_saved_components(
        &self,
        data_version: i32,
        item: &str,
        count: u32,
        components: Vec<(String, String)>,
    ) -> Option<ItemStack> {
        use horizon_world::datafix;
        if !(0..=datafix::CURRENT).contains(&data_version)
            || !(1..=99).contains(&count)
            || mod_api::Identifier::parse(item).is_err()
        {
            return None;
        }
        let mut available = MAX_SNBT_BYTES.checked_sub(item.len())?;
        let mut patch = Compound::new();
        for (id, value) in components {
            let name = id.strip_prefix('!').unwrap_or(&id);
            if mod_api::Identifier::parse(name).is_err() {
                return None;
            }
            available = available.checked_sub(id.len())?.checked_sub(value.len())?;
            let (tag, end) = horizon_core::nbt::parse_snbt(&value).ok()?;
            if !value.as_bytes()[end..].iter().all(u8::is_ascii_whitespace)
                || patch.insert(id.as_str().into(), tag).is_some()
            {
                return None;
            }
        }
        let mut input = Compound::from([
            ("id".into(), Tag::String(item.into())),
            ("count".into(), Tag::Int(count as i32)),
        ]);
        if !patch.is_empty() {
            input.insert("components".into(), Tag::Compound(patch));
        }
        self.upgraded_saved_stack(data_version, input, count)
    }

    pub(super) fn item_from_saved_gzip_nbt(&self, bytes: Vec<u8>) -> Option<ItemStack> {
        if bytes.len() > MAX_SAVED_GZIP_BYTES {
            return None;
        }
        let mut gzip = flate2::bufread::GzDecoder::new(bytes.as_slice());
        let mut raw = Vec::new();
        gzip.by_ref()
            .take((MAX_SAVED_NBT_BYTES + 1) as u64)
            .read_to_end(&mut raw)
            .ok()?;
        if raw.len() > MAX_SAVED_NBT_BYTES || !gzip.get_ref().is_empty() {
            return None;
        }
        let (root, consumed) = nbt::read_named(&raw, saved_nbt_limits()).ok()?;
        let Tag::Compound(mut input) = root.tag else {
            return None;
        };
        if consumed != raw.len() {
            return None;
        }
        let Tag::Int(data_version) = input.remove(&"DataVersion".into())? else {
            return None;
        };
        let count = match input
            .get(&"count".into())
            .or_else(|| input.get(&"Count".into()))
        {
            None => 1,
            Some(Tag::Byte(n)) => i32::from(*n),
            Some(Tag::Short(n)) => i32::from(*n),
            Some(Tag::Int(n)) => *n,
            _ => return None,
        };
        if !(1..=99).contains(&count) {
            return None;
        }
        self.upgraded_saved_stack(data_version, input, count as u32)
    }

    pub(super) fn item_to_saved_gzip_nbt(&self, stack: &ItemStack) -> Option<Vec<u8>> {
        if !(1..=99).contains(&stack.count)
            || stack.item.as_ref() == "minecraft:air"
            || self.items().get(&stack.item).is_none()
        {
            return None;
        }
        let Tag::Compound(mut item) =
            horizon_items::item_nbt::item_to_nbt_with_items(stack, self.items())
        else {
            return None;
        };
        item.insert(
            "DataVersion".into(),
            Tag::Int(horizon_core::version::DATA_VERSION),
        );
        let bytes = nbt::write_named_strict(
            &nbt::NamedTag {
                name: Default::default(),
                tag: Tag::Compound(item),
            },
            saved_nbt_limits(),
        )
        .ok()?;
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&bytes).ok()?;
        let compressed = gzip.finish().ok()?;
        (compressed.len() <= MAX_SAVED_GZIP_BYTES).then_some(compressed)
    }

    fn upgraded_saved_stack(
        &self,
        data_version: i32,
        input: Compound,
        count: u32,
    ) -> Option<ItemStack> {
        let fixed = horizon_world::datafix::upgrade_item_stack(data_version, input).ok()?;
        if fixed
            .get(&"components".into())
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return None;
        }
        let fixed = Tag::Compound(fixed);
        let stack = horizon_items::item_nbt::item_from_nbt(&fixed)?;
        (stack.count == count && self.items().get(&stack.item).is_some()).then_some(stack)
    }
}

fn saved_nbt_limits() -> Limits {
    Limits {
        max_bytes: MAX_SAVED_NBT_BYTES,
        ..Limits::default()
    }
}
