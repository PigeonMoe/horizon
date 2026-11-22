//! Stage a Paper 26.2 PM-Sync snapshot against a native destination player.
//! The caller owns authorization, the receipt, and final gameplay application.

use std::{
    collections::{BTreeMap, BTreeSet},
    convert::Infallible,
    fmt,
};

use horizon_client_core::{
    nbt_value::{float, string, text},
    potions,
};
use horizon_core::{
    identifier::Identifier,
    nbt::{self, Compound, Limits, NamedTag, Tag},
    version::DATA_VERSION,
};
use horizon_items::{
    equipment,
    husksync_effects_and_attributes::HuskAttribute,
    husksync_inventory::NativePlayerItems,
    husksync_packed::{BukkitMinecraftVersion, PackedCompression, PackedLimits, PackedSnapshot},
    husksync_records::{HuskPlayerRecords, HuskStatistics},
    husksync_vitals::HuskHealth,
    item_nbt::item_to_nbt_with_items,
    items::{ItemStack, Items},
};
use horizon_server_files::pm_sync_envelope::SnapshotEnvelope;
use horizon_world::attributes::{
    Definition, Instance, Modifier, Observer, Operation, Packed, entity::Catalog,
};

use crate::recipe_book::{RecipeIndex, Settings};

const PLAYER: &str = "minecraft:player";
const MAX_HEALTH: &str = "minecraft:max_health";

pub struct PaperMigrationContext<'a> {
    pub items: &'a Items,
    pub attributes: &'a Catalog,
    pub recipes: &'a RecipeIndex,
    pub compression: PackedCompression,
    pub limits: PackedLimits,
}

#[derive(Debug, PartialEq)]
pub struct StagedPaperMigration<'a> {
    pub player: Compound,
    pub health: PaperMigrationHealth,
    /// Paper rewrites awarded criteria in known destination advancement
    /// definitions. Award times are Java epoch milliseconds.
    pub advancements: BTreeMap<Identifier, BTreeMap<String, i64>>,
    /// Paper updates the counters present in the source and keeps other counters.
    pub statistics: HuskStatistics,
    /// External participants validate their own sections and owner binding.
    pub sections: BTreeMap<String, &'a [u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaperMigrationHealth {
    /// The inner Husk health, after clamping to the projected destination maximum.
    pub initial: f32,
    /// The original Bukkit health and display-scale values for a capable caller.
    pub husk: HuskHealth,
    /// The envelope health used after PM-Attibute recalculates its modifiers.
    pub after_bridges: f64,
}

#[derive(Debug)]
pub enum PaperMigrationError {
    Destination(&'static str),
    Snapshot(String),
    SourceVersion,
    Attribute(String),
    AttributeBounds(String),
    Effect(String),
    Item,
    Health,
    NativeNbt(nbt::Error),
}

impl fmt::Display for PaperMigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Destination(key) => write!(f, "invalid destination player field {key}"),
            Self::Snapshot(error) => f.write_str(error),
            Self::SourceVersion => f.write_str("expected the Paper 26.2 PM-Sync snapshot version"),
            Self::Attribute(id) => write!(f, "invalid or unsupported player attribute {id:?}"),
            Self::AttributeBounds(id) => {
                write!(f, "the native loader changes the effective value of {id:?}")
            }
            Self::Effect(id) => write!(f, "unknown destination potion effect {id:?}"),
            Self::Item => f.write_str("could not encode a migrated item in native NBT"),
            Self::Health => f.write_str("invalid health for a staged player arrival"),
            Self::NativeNbt(error) => {
                write!(f, "migrated player cannot be saved as native NBT: {error}")
            }
        }
    }
}

impl std::error::Error for PaperMigrationError {}

/// Validate the whole source and produce a native player compound without
/// changing either input. The destination compound is current native data
/// for the authenticated owner, typically `playerdata::player_tag`.
pub fn stage_paper_migration<'a>(
    envelope: &SnapshotEnvelope<'a>,
    destination: &Compound,
    context: &PaperMigrationContext<'_>,
) -> Result<StagedPaperMigration<'a>, PaperMigrationError> {
    if destination.get(&"DataVersion".into()) != Some(&Tag::Int(DATA_VERSION)) {
        return Err(PaperMigrationError::Destination("DataVersion"));
    }
    if !matches!(destination.get(&"UUID".into()), Some(Tag::IntArray(id)) if id.len() == 4) {
        return Err(PaperMigrationError::Destination("UUID"));
    }
    if !envelope.health.is_finite() || envelope.health < 0.0 {
        return Err(PaperMigrationError::Health);
    }

    let packed = PackedSnapshot::decode(
        envelope.husk,
        context.compression,
        BukkitMinecraftVersion::V26_3,
        context.limits,
    )
    .map_err(snapshot_error)?;
    if packed.source_version() != BukkitMinecraftVersion::V26_2 {
        return Err(PaperMigrationError::SourceVersion);
    }
    packed.validate_pm_sync_fields().map_err(snapshot_error)?;
    let inventory = packed
        .inventory(context.limits.max_json_bytes)
        .map_err(snapshot_error)?
        .to_native(context.items)
        .map_err(snapshot_error)?;
    let vitals = packed.startup_vitals().map_err(snapshot_error)?;
    let transient = packed
        .startup_effects_and_attributes()
        .map_err(snapshot_error)?;
    let HuskPlayerRecords {
        persistent_data,
        advancements,
        statistics,
    } = packed
        .player_records(context.limits.max_json_bytes)
        .map_err(snapshot_error)?;

    let mut player = copy_destination(destination)?;
    let (attributes, maximum) =
        project_attributes(destination, &transient.attributes, context.attributes)?;
    let initial = vitals.health.health.min(f64::from(maximum)) as f32;
    if !initial.is_finite() || initial <= 0.0 {
        return Err(PaperMigrationError::Health);
    }
    let effects = project_effects(&transient.effects)?;
    let recipe_book = project_recipes(destination, envelope, context.recipes)?;

    store_inventory(&mut player, &inventory, context.items)?;
    player.extend([
        ("Health".into(), Tag::Float(initial.to_bits())),
        ("foodLevel".into(), Tag::Int(vitals.hunger.food_level)),
        (
            "foodSaturationLevel".into(),
            Tag::Float(vitals.hunger.saturation.to_bits()),
        ),
        (
            "foodExhaustionLevel".into(),
            Tag::Float(vitals.hunger.exhaustion.to_bits()),
        ),
        ("XpLevel".into(), Tag::Int(vitals.experience.exp_level)),
        (
            "XpTotal".into(),
            Tag::Int(vitals.experience.total_experience),
        ),
        (
            "XpP".into(),
            Tag::Float(vitals.experience.exp_progress.to_bits()),
        ),
        ("recipeBook".into(), recipe_book),
    ]);
    set_list(&mut player, "attributes", attributes);
    set_list(&mut player, "active_effects", effects);
    if persistent_data.is_empty() {
        player.remove(&"BukkitValues".into());
    } else {
        player.insert("BukkitValues".into(), Tag::Compound(persistent_data));
    }
    let named = NamedTag {
        name: Default::default(),
        tag: Tag::Compound(player),
    };
    nbt::write_named_strict(&named, Limits::saved()).map_err(PaperMigrationError::NativeNbt)?;
    let Tag::Compound(player) = named.tag else {
        unreachable!("native player compound")
    };
    Ok(StagedPaperMigration {
        player,
        health: PaperMigrationHealth {
            initial,
            husk: vitals.health,
            after_bridges: envelope.health,
        },
        advancements,
        statistics,
        sections: envelope.sections.clone(),
    })
}

fn snapshot_error(error: impl fmt::Display) -> PaperMigrationError {
    PaperMigrationError::Snapshot(error.to_string())
}

fn copy_destination(destination: &Compound) -> Result<Compound, PaperMigrationError> {
    let max_depth = Limits::saved().max_depth - 1;
    if destination
        .values()
        .any(|value| !nbt::within_depth(value, max_depth))
    {
        return Err(PaperMigrationError::Destination("NBT depth"));
    }
    Ok(destination
        .iter()
        .map(|(key, value)| (key.clone(), nbt::clone_tag(value)))
        .collect())
}

fn item_tag(stack: &ItemStack, items: &Items) -> Result<Compound, PaperMigrationError> {
    match item_to_nbt_with_items(stack, items) {
        Tag::Compound(item) => Ok(item),
        _ => Err(PaperMigrationError::Item),
    }
}

fn container_tag(slots: &[Option<ItemStack>], items: &Items) -> Result<Tag, PaperMigrationError> {
    let mut entries = Vec::new();
    for (slot, item) in slots.iter().enumerate() {
        if let Some(item) = item {
            let mut item = item_tag(item, items)?;
            item.insert("Slot".into(), Tag::Byte(slot as i8));
            entries.push(Tag::Compound(item));
        }
    }
    Ok(Tag::List(entries))
}

fn store_inventory(
    player: &mut Compound,
    inventory: &NativePlayerItems,
    items: &Items,
) -> Result<(), PaperMigrationError> {
    player.insert(
        "Inventory".into(),
        container_tag(&inventory.inventory[..36], items)?,
    );
    player.insert(
        "EnderItems".into(),
        container_tag(&inventory.ender_chest, items)?,
    );
    player.insert(
        "SelectedItemSlot".into(),
        Tag::Int(i32::from(inventory.held_item_slot)),
    );
    let mut worn = match player.remove(&"equipment".into()) {
        Some(Tag::Compound(old)) => old,
        None => Compound::new(),
        Some(_) => return Err(PaperMigrationError::Destination("equipment")),
    };
    let ordinals = [
        equipment::FEET,
        equipment::LEGS,
        equipment::CHEST,
        equipment::HEAD,
        equipment::OFFHAND,
    ];
    for (ordinal, item) in ordinals.into_iter().zip(&inventory.inventory[36..]) {
        let slot = equipment::SLOT_NAMES[ordinal].into();
        match item {
            Some(item) => {
                worn.insert(slot, Tag::Compound(item_tag(item, items)?));
            }
            None => {
                worn.remove(&slot);
            }
        }
    }
    if !worn.is_empty() || inventory.inventory[usize::from(inventory.held_item_slot)].is_some() {
        player.insert("equipment".into(), Tag::Compound(worn));
    }
    Ok(())
}

fn set_list(player: &mut Compound, key: &str, entries: Vec<Tag>) {
    if entries.is_empty() {
        player.remove(&key.into());
    } else {
        player.insert(key.into(), Tag::List(entries));
    }
}

fn project_effects(
    effects: &[horizon_items::husksync_effects_and_attributes::HuskPotionEffect],
) -> Result<Vec<Tag>, PaperMigrationError> {
    effects
        .iter()
        .map(|effect| {
            if potions::effect(&effect.effect_type).is_none() {
                return Err(PaperMigrationError::Effect(effect.effect_type.clone()));
            }
            let mut fields = vec![
                ("id", string(&effect.effect_type)),
                ("duration", Tag::Int(effect.duration)),
                ("show_icon", Tag::Byte(i8::from(effect.has_icon))),
            ];
            // `MobEffectInstance.Details` writes an unsigned amplifier byte and
            // omits the default particle and ambient flags.
            if effect.amplifier != 0 {
                fields.push(("amplifier", Tag::Byte(effect.amplifier as u8 as i8)));
            }
            if !effect.show_particles {
                fields.push(("show_particles", Tag::Byte(0)));
            }
            if effect.is_ambient {
                fields.push(("ambient", Tag::Byte(1)));
            }
            Ok(Tag::compound(fields))
        })
        .collect()
}

fn project_recipes(
    destination: &Compound,
    envelope: &SnapshotEnvelope<'_>,
    index: &RecipeIndex,
) -> Result<Tag, PaperMigrationError> {
    let original = destination.get(&"recipeBook".into());
    let mut result = match original {
        Some(Tag::Compound(book)) => book.clone(),
        None => Compound::new(),
        Some(_) => return Err(PaperMigrationError::Destination("recipeBook")),
    };
    let mut settings = Settings::read(original);
    let wanted: BTreeSet<_> = envelope
        .recipes
        .iter()
        .filter_map(|entry| entry.to_string().ok())
        .filter_map(|entry| Identifier::parse(&entry).ok())
        .filter(|entry| !entry.path().is_empty())
        .map(|entry| entry.to_string())
        .filter(|entry| index.contains(entry))
        .collect();
    let removed = settings
        .recipes
        .difference(&wanted)
        .cloned()
        .collect::<Vec<_>>();
    settings.remove_recipes(index, &removed);
    settings
        .to_be_displayed
        .retain(|id| settings.recipes.contains(id));
    settings.add_recipes(index, &wanted.into_iter().collect::<Vec<_>>());
    let Tag::Compound(written) = settings.write() else {
        unreachable!("native recipe book")
    };
    for name in ["recipes", "toBeDisplayed"] {
        result.insert(name.into(), written[&name.into()].clone());
    }
    Ok(Tag::Compound(result))
}

fn player_attribute(catalog: &Catalog, name: &str) -> Option<(Definition, f64)> {
    let id = catalog.id(name)?;
    Some((
        catalog.attribute(id).definition,
        catalog.default_base(PLAYER, id)?,
    ))
}

fn project_attributes(
    destination: &Compound,
    incoming: &[HuskAttribute],
    catalog: &Catalog,
) -> Result<(Vec<Tag>, f32), PaperMigrationError> {
    let local = match destination.get(&"attributes".into()) {
        Some(Tag::List(entries)) => entries.as_slice(),
        None => &[],
        Some(_) => return Err(PaperMigrationError::Destination("attributes")),
    };
    let (definition, default) =
        player_attribute(catalog, MAX_HEALTH).ok_or(PaperMigrationError::Health)?;
    let mut maximum = definition.sanitize(default);
    let mut projected = BTreeMap::new();
    for entry in local {
        let name = text(entry.get("id")).ok_or(PaperMigrationError::Destination("attribute id"))?;
        let updated = if name.starts_with("minecraft:") {
            if let Some((definition, _)) = player_attribute(catalog, &name) {
                // Paper first clears the old modifiers. With no incoming row
                // it keeps the post-removal effective value as the new base.
                let base = float(entry.get("base"))
                    .filter(|base| base.is_finite())
                    .ok_or(PaperMigrationError::Destination("attribute base"))?;
                let base = definition.sanitize(base);
                if name == MAX_HEALTH {
                    maximum = base;
                }
                attribute_entry(&name, base, &[], Some(entry))
            } else {
                nbt::clone_tag(entry)
            }
        } else {
            nbt::clone_tag(entry)
        };
        if projected.insert(name, updated).is_some() {
            return Err(PaperMigrationError::Destination("duplicate attribute id"));
        }
    }
    for attribute in incoming {
        let name = &attribute.name;
        let (definition, _) = player_attribute(catalog, name)
            .filter(|_| name.starts_with("minecraft:"))
            .ok_or_else(|| PaperMigrationError::Attribute(name.clone()))?;
        let modifiers = attribute
            .modifiers
            .iter()
            .map(|modifier| {
                let operation = modifier
                    .native_operation()
                    .ok_or_else(|| PaperMigrationError::Attribute(name.clone()))?;
                Modifier::new(&modifier.name, modifier.amount, operation)
                    .map_err(|_| PaperMigrationError::Attribute(name.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let base = attribute.base_value;
        let expected = attribute_value(definition, base, &modifiers)?;
        if !expected.is_finite() {
            return Err(PaperMigrationError::Attribute(name.clone()));
        }
        // Horizon clamps persisted bases when their configured bounds shrink.
        // Preserve the actual source base when that load has the same result.
        let loaded_base = definition.sanitize(base);
        if base != loaded_base && expected != attribute_value(definition, loaded_base, &modifiers)?
        {
            return Err(PaperMigrationError::AttributeBounds(name.clone()));
        }
        if name == MAX_HEALTH {
            maximum = expected;
        }
        let entry = attribute_entry(name, base, &modifiers, projected.get(name));
        projected.insert(name.clone(), entry);
    }
    let maximum = maximum as f32;
    if !maximum.is_finite() || maximum <= 0.0 {
        return Err(PaperMigrationError::Health);
    }
    Ok((projected.into_values().collect(), maximum))
}

fn attribute_entry(name: &str, base: f64, modifiers: &[Modifier], local: Option<&Tag>) -> Tag {
    let mut entry = match local {
        Some(Tag::Compound(local)) => local.clone(),
        _ => Compound::new(),
    };
    entry.insert("id".into(), string(name));
    entry.insert("base".into(), Tag::Double(base.to_bits()));
    let modifiers = modifiers
        .iter()
        .map(|modifier| {
            let operation = match modifier.operation() {
                Operation::AddValue => "add_value",
                Operation::AddMultipliedBase => "add_multiplied_base",
                Operation::AddMultipliedTotal => "add_multiplied_total",
            };
            Tag::compound([
                ("id", string(modifier.id())),
                ("amount", Tag::Double(modifier.amount().to_bits())),
                ("operation", string(operation)),
            ])
        })
        .collect();
    set_list(&mut entry, "modifiers", modifiers);
    Tag::Compound(entry)
}

fn attribute_value(
    definition: Definition,
    base: f64,
    modifiers: &[Modifier],
) -> Result<f64, PaperMigrationError> {
    struct Detached;
    impl Observer for Detached {
        type Error = Infallible;
        fn dirty(&mut self, _: &mut Instance) -> Result<(), Self::Error> {
            Ok(())
        }
    }
    let mut instance = Instance::new(definition);
    instance
        .apply(
            &Packed {
                base,
                modifiers: modifiers.to_vec(),
            },
            &mut Detached,
        )
        .map_err(snapshot_error)?;
    Ok(instance.value())
}

#[cfg(test)]
#[path = "pm_sync_paper_tests.rs"]
mod tests;
