//! The inventory and ender-chest SNBT fields in a Bukkit HuskSync 4.0.0 snapshot.

use std::fmt;

use horizon_core::{
    nbt::{self, Compound, Limits, NamedTag, Tag},
    version::DATA_VERSION,
};
use horizon_world::{datafix::upgrade_item_stack, snbt_text::to_snbt};

use crate::{
    item_nbt::{item_from_nbt, item_to_nbt_with_items},
    items::{ItemStack, Items},
};

pub const INVENTORY_KEY: &str = "husksync:inventory";
pub const ENDER_CHEST_KEY: &str = "husksync:ender_chest";
pub const INVENTORY_SLOTS: usize = 41;
pub const ENDER_CHEST_SLOTS: usize = 27;

#[derive(Clone, Debug, PartialEq)]
pub struct NativePlayerItems {
    /// Bukkit order: hotbar 0–8, main storage 9–35, feet 36, legs 37,
    /// chest 38, head 39, offhand 40.
    pub inventory: [Option<ItemStack>; INVENTORY_SLOTS],
    pub held_item_slot: u8,
    pub ender_chest: [Option<ItemStack>; ENDER_CHEST_SLOTS],
}

#[derive(Debug, PartialEq, Eq)]
pub struct InventoryFields {
    pub inventory: String,
    pub ender_chest: String,
}

/// A validated pair of saved fields. Keeping the source NBT and its versions
/// permits re-emitting a Paper snapshot even after projecting it into Horizon.
#[derive(Debug)]
pub struct HuskPlayerItems {
    source_version: i32,
    inventory: [Option<SavedItem>; INVENTORY_SLOTS],
    held_item_slot: u8,
    ender_chest: [Option<SavedItem>; ENDER_CHEST_SLOTS],
}

#[derive(Debug)]
struct SavedItem {
    version: i32,
    stack: Compound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InventoryError {
    MissingField(&'static str),
    InvalidField(&'static str),
    InvalidHeldSlot,
    InvalidSlot,
    InvalidStack,
    Version,
    SizeLimit,
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(key) => write!(f, "missing HuskSync field {key}"),
            Self::InvalidField(key) => write!(f, "invalid HuskSync field {key}"),
            Self::InvalidHeldSlot => f.write_str("invalid HuskSync held hotbar slot"),
            Self::InvalidSlot => f.write_str("invalid or repeated HuskSync item slot"),
            Self::InvalidStack => f.write_str("invalid HuskSync item stack"),
            Self::Version => f.write_str("incompatible HuskSync item data version"),
            Self::SizeLimit => f.write_str("HuskSync item field exceeds its size limit"),
        }
    }
}

impl std::error::Error for InventoryError {}

impl HuskPlayerItems {
    /// Parse both required keys. In a generated empty field, `size` is present
    /// and `items` can be empty or absent. `source_version` comes from the
    /// enclosing snapshot's trusted Minecraft version mapping.
    pub fn parse(
        inventory: Option<&str>,
        ender_chest: Option<&str>,
        source_version: i32,
        max_field_bytes: usize,
    ) -> Result<Self, InventoryError> {
        if !(1..=DATA_VERSION).contains(&source_version) {
            return Err(InventoryError::Version);
        }
        let mut inventory = parse_field(inventory, INVENTORY_KEY, max_field_bytes)?;
        let ender_chest = parse_field(ender_chest, ENDER_CHEST_KEY, max_field_bytes)?;
        let held_item_slot = match inventory.remove(&"held_item_slot".into()) {
            Some(Tag::Int(slot @ 0..=8)) => slot as u8,
            _ => return Err(InventoryError::InvalidHeldSlot),
        };
        let Some(Tag::Compound(inventory_items)) = inventory.remove(&"items".into()) else {
            return Err(InventoryError::InvalidField(INVENTORY_KEY));
        };
        if !inventory.is_empty() {
            return Err(InventoryError::InvalidField(INVENTORY_KEY));
        }
        Ok(Self {
            source_version,
            inventory: parse_slots(inventory_items, source_version, INVENTORY_KEY)?,
            held_item_slot,
            ender_chest: parse_slots(ender_chest, source_version, ENDER_CHEST_KEY)?,
        })
    }

    /// Stage a complete native projection before a caller applies either field.
    pub fn to_native(&self, items: &Items) -> Result<NativePlayerItems, InventoryError> {
        let upgrade = |saved: &SavedItem| {
            let upgraded = upgrade_item_stack(saved.version, saved.stack.clone())
                .map_err(|_| InventoryError::InvalidStack)?;
            decode_stack(upgraded, items)
        };
        Ok(NativePlayerItems {
            inventory: map_slots(&self.inventory, upgrade)?,
            held_item_slot: self.held_item_slot,
            ender_chest: map_slots(&self.ender_chest, upgrade)?,
        })
    }

    /// Capture edited native fields at Horizon's real current DataVersion.
    pub fn from_native(native: &NativePlayerItems, items: &Items) -> Result<Self, InventoryError> {
        if native.held_item_slot > 8 {
            return Err(InventoryError::InvalidHeldSlot);
        }
        Ok(Self {
            source_version: DATA_VERSION,
            inventory: map_slots(&native.inventory, |stack| save_stack(stack, items))?,
            held_item_slot: native.held_item_slot,
            ender_chest: map_slots(&native.ender_chest, |stack| save_stack(stack, items))?,
        })
    }

    pub fn source_data_version(&self) -> i32 {
        self.source_version
    }

    /// Emit the Paper field shapes and retain each item's original version. A
    /// target older than the source needs a separately verified downgrade.
    pub fn encode(
        &self,
        target_data_version: i32,
        max_field_bytes: usize,
    ) -> Result<InventoryFields, InventoryError> {
        if target_data_version < self.source_version {
            return Err(InventoryError::Version);
        }
        Ok(InventoryFields {
            inventory: encode_field(
                Tag::compound([
                    ("items", encode_slots(&self.inventory)),
                    ("held_item_slot", Tag::Int(i32::from(self.held_item_slot))),
                ]),
                INVENTORY_KEY,
                max_field_bytes,
            )?,
            ender_chest: encode_field(
                encode_slots(&self.ender_chest),
                ENDER_CHEST_KEY,
                max_field_bytes,
            )?,
        })
    }
}

fn parse_field(
    text: Option<&str>,
    key: &'static str,
    max_bytes: usize,
) -> Result<Compound, InventoryError> {
    let text = text.ok_or(InventoryError::MissingField(key))?;
    if text.len() > max_bytes {
        return Err(InventoryError::SizeLimit);
    }
    let (Tag::Compound(field), used) =
        nbt::parse_snbt(text).map_err(|_| InventoryError::InvalidField(key))?
    else {
        return Err(InventoryError::InvalidField(key));
    };
    if !text[used..].chars().all(|c| nbt::snbt_whitespace(c as u32)) {
        return Err(InventoryError::InvalidField(key));
    }
    Ok(field)
}

fn parse_slots<const N: usize>(
    mut field: Compound,
    source_version: i32,
    key: &'static str,
) -> Result<[Option<SavedItem>; N], InventoryError> {
    if field.remove(&"size".into()) != Some(Tag::Int(N as i32)) {
        return Err(InventoryError::InvalidField(key));
    }
    let entries = match field.remove(&"items".into()) {
        None => Vec::new(),
        Some(Tag::List(entries)) if entries.len() <= N => entries,
        _ => return Err(InventoryError::InvalidField(key)),
    };
    if !field.is_empty() {
        return Err(InventoryError::InvalidField(key));
    }
    let mut slots: [Option<SavedItem>; N] = std::array::from_fn(|_| None);
    for entry in entries {
        let Tag::Compound(mut stack) = entry else {
            return Err(InventoryError::InvalidStack);
        };
        let Some(Tag::Int(slot)) = stack.remove(&"Slot".into()) else {
            return Err(InventoryError::InvalidSlot);
        };
        let cell = slots
            .get_mut(slot as usize)
            .ok_or(InventoryError::InvalidSlot)?;
        if cell.is_some() {
            return Err(InventoryError::InvalidSlot);
        }
        let version = match stack.remove(&"DataVersion".into()) {
            Some(Tag::Int(version)) if (1..=source_version).contains(&version) => version,
            _ => return Err(InventoryError::Version),
        };
        validate_stack(&stack)?;
        *cell = Some(SavedItem { version, stack });
    }
    Ok(slots)
}

fn validate_stack(stack: &Compound) -> Result<u32, InventoryError> {
    let Some(Tag::String(id)) = stack.get(&"id".into()) else {
        return Err(InventoryError::InvalidStack);
    };
    let id = id.to_string().map_err(|_| InventoryError::InvalidStack)?;
    if horizon_world::datafix::identifier(&id).is_none_or(|(namespace, path)| {
        path.is_empty() || (namespace == "minecraft" && path == "air")
    }) || stack
        .get(&"components".into())
        .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
    {
        return Err(InventoryError::InvalidStack);
    }
    let count = match stack
        .get(&"count".into())
        .or_else(|| stack.get(&"Count".into()))
    {
        None => 1,
        Some(Tag::Byte(n)) => i32::from(*n),
        Some(Tag::Short(n)) => i32::from(*n),
        Some(Tag::Int(n)) => *n,
        _ => return Err(InventoryError::InvalidStack),
    };
    if !(1..=99).contains(&count) {
        return Err(InventoryError::InvalidStack);
    }
    Ok(count as u32)
}

fn decode_stack(stack: Compound, items: &Items) -> Result<ItemStack, InventoryError> {
    let count = validate_stack(&stack)?;
    if stack.keys().any(|key| {
        !matches!(
            key.to_string_lossy().as_str(),
            "id" | "count" | "components"
        )
    }) {
        return Err(InventoryError::InvalidStack);
    }
    item_from_nbt(&Tag::Compound(stack))
        .filter(|stack| stack.count == count && items.get(&stack.item).is_some())
        .ok_or(InventoryError::InvalidStack)
}

fn save_stack(stack: &ItemStack, items: &Items) -> Result<SavedItem, InventoryError> {
    if !(1..=99).contains(&stack.count)
        || stack.item.as_ref() == "minecraft:air"
        || items.get(&stack.item).is_none()
    {
        return Err(InventoryError::InvalidStack);
    }
    let Tag::Compound(stack) = item_to_nbt_with_items(stack, items) else {
        return Err(InventoryError::InvalidStack);
    };
    Ok(SavedItem {
        version: DATA_VERSION,
        stack,
    })
}

fn map_slots<T, U, const N: usize>(
    input: &[Option<T>; N],
    convert: impl Fn(&T) -> Result<U, InventoryError>,
) -> Result<[Option<U>; N], InventoryError> {
    let mut output = std::array::from_fn(|_| None);
    for (source, target) in input.iter().zip(&mut output) {
        *target = source.as_ref().map(&convert).transpose()?;
    }
    Ok(output)
}

fn encode_slots<const N: usize>(slots: &[Option<SavedItem>; N]) -> Tag {
    let mut entries = Vec::new();
    for (slot, item) in slots.iter().enumerate() {
        if let Some(item) = item {
            let mut stack = item.stack.clone();
            stack.insert("Slot".into(), Tag::Int(slot as i32));
            stack.insert("DataVersion".into(), Tag::Int(item.version));
            entries.push(Tag::Compound(stack));
        }
    }
    let mut field = Compound::from([("size".into(), Tag::Int(N as i32))]);
    if !entries.is_empty() {
        field.insert("items".into(), Tag::List(entries));
    }
    Tag::Compound(field)
}

fn encode_field(tag: Tag, key: &'static str, max_bytes: usize) -> Result<String, InventoryError> {
    // Cap the encoded tree before expanding array entries and escapes into text.
    let limits = Limits {
        max_bytes: max_bytes.saturating_mul(4).saturating_add(64),
        ..Limits::default()
    };
    let root = NamedTag {
        name: Default::default(),
        tag,
    };
    nbt::write_named_strict(&root, limits).map_err(|_| InventoryError::SizeLimit)?;
    let text = to_snbt(&root.tag);
    let rebuilt = Tag::Compound(parse_field(Some(&text), key, max_bytes)?);
    if rebuilt != root.tag {
        return Err(InventoryError::InvalidField(key));
    }
    Ok(text)
}
