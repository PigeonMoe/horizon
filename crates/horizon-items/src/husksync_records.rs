//! Paper HuskSync persistent data, advancements and statistics in a Packed snapshot.

use std::{collections::BTreeMap, fmt};

use horizon_core::{
    identifier::Identifier,
    nbt::{self, Compound, Tag},
};
use serde::Deserialize;

use crate::husksync_packed::{PackedSnapshot, UniqueMap};

const PERSISTENT_DATA: &str = "husksync:persistent_data";
const ADVANCEMENTS: &str = "husksync:advancements";
const STATISTICS: &str = "husksync:statistics";
const REQUIRED: [&str; 10] = [
    "husksync:inventory",
    "husksync:ender_chest",
    PERSISTENT_DATA,
    "husksync:health",
    "husksync:experience",
    "husksync:attributes",
    "husksync:hunger",
    ADVANCEMENTS,
    STATISTICS,
    "husksync:potion_effects",
];
const LOCAL_ONLY: [&str; 3] = [
    "husksync:location",
    "husksync:game_mode",
    "husksync:flight_status",
];

#[derive(Debug, PartialEq)]
pub struct HuskPlayerRecords {
    /// The player's Paper `BukkitValues` compound, preserving NBT types.
    pub persistent_data: Compound,
    /// Awarded criteria keyed by advancement. Values are Java epoch milliseconds.
    pub advancements: BTreeMap<Identifier, BTreeMap<String, i64>>,
    pub statistics: HuskStatistics,
}

/// The original Bukkit maps contain only the values present in the snapshot.
/// A destination overwrites those counters and retains other local counters.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct HuskStatistics {
    pub generic: BTreeMap<String, i32>,
    pub blocks: BTreeMap<String, BTreeMap<String, i32>>,
    pub items: BTreeMap<String, BTreeMap<String, i32>>,
    pub entities: BTreeMap<String, BTreeMap<String, i32>>,
}

impl HuskStatistics {
    /// Apply the counters Paper would recognize to a copy of the destination's stats.
    /// The caller resolves the vanilla type and value against its actual registries.
    pub fn overlay_native(
        &self,
        destination: &mut BTreeMap<String, BTreeMap<String, i32>>,
        mut supported: impl FnMut(&str, &str) -> bool,
    ) {
        let mut set = |kind: &str, key: &str, value: i32| {
            if value < 0 {
                return;
            }
            let key = format!("minecraft:{key}");
            if supported(kind, &key) {
                destination
                    .entry(kind.to_owned())
                    .or_default()
                    .insert(key, value);
            }
        };
        for (id, &value) in &self.generic {
            set("minecraft:custom", vanilla_custom_stat(id), value);
        }
        for (paper, vanilla, values) in [
            ("mine_block", "minecraft:mined", &self.blocks),
            ("break_item", "minecraft:broken", &self.items),
            ("craft_item", "minecraft:crafted", &self.items),
            ("drop", "minecraft:dropped", &self.items),
            ("pickup", "minecraft:picked_up", &self.items),
            ("use_item", "minecraft:used", &self.items),
            ("kill_entity", "minecraft:killed", &self.entities),
            ("entity_killed_by", "minecraft:killed_by", &self.entities),
        ] {
            if let Some(counters) = values.get(paper) {
                for (key, &value) in counters {
                    set(vanilla, key, value);
                }
            }
        }
    }
}

// Bukkit's enum keys use legacy names for these untyped statistics.
// Source: Paper 26.2 CraftStatistic, revision 19ebc4a.
fn vanilla_custom_stat(id: &str) -> &str {
    match id {
        "armor_cleaned" => "clean_armor",
        "banner_cleaned" => "clean_banner",
        "drop_count" => "drop",
        "cake_slices_eaten" => "eat_cake_slice",
        "item_enchanted" => "enchant_item",
        "cauldron_filled" => "fill_cauldron",
        "dispenser_inspected" => "inspect_dispenser",
        "dropper_inspected" => "inspect_dropper",
        "hopper_inspected" => "inspect_hopper",
        "beacon_interaction" => "interact_with_beacon",
        "brewingstand_interaction" => "interact_with_brewingstand",
        "crafting_table_interaction" => "interact_with_crafting_table",
        "furnace_interaction" => "interact_with_furnace",
        "chest_opened" => "open_chest",
        "enderchest_opened" => "open_enderchest",
        "shulker_box_opened" => "open_shulker_box",
        "noteblock_played" => "play_noteblock",
        "record_played" => "play_record",
        "play_one_minute" => "play_time",
        "flower_potted" => "pot_flower",
        "sneak_time" => "crouch_time",
        "trapped_chest_triggered" => "trigger_trapped_chest",
        "noteblock_tuned" => "tune_noteblock",
        "cauldron_used" => "use_cauldron",
        id => id,
    }
}

#[derive(Debug)]
pub enum RecordsError {
    Missing(&'static str),
    LocalField(&'static str),
    SizeLimit(&'static str),
    Json(&'static str, serde_json::Error),
    PersistentData,
    AdvancementKey,
    DuplicateAdvancement,
}

impl fmt::Display for RecordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(key) => write!(f, "missing HuskSync field {key}"),
            Self::LocalField(key) => write!(f, "PM-Sync must keep {key} local"),
            Self::SizeLimit(key) => write!(f, "HuskSync field {key} exceeds its size limit"),
            Self::Json(key, error) => write!(f, "invalid HuskSync field {key}: {error}"),
            Self::PersistentData => f.write_str("invalid HuskSync player persistent data"),
            Self::AdvancementKey => f.write_str("invalid HuskSync advancement key"),
            Self::DuplicateAdvancement => f.write_str("repeated HuskSync advancement"),
        }
    }
}

impl std::error::Error for RecordsError {}

#[derive(Deserialize)]
struct Advancement {
    key: String,
    completed_criteria: UniqueMap<i64>,
}

#[derive(Deserialize)]
struct Statistics {
    generic: UniqueMap<i32>,
    blocks: UniqueMap<UniqueMap<i32>>,
    items: UniqueMap<UniqueMap<i32>>,
    entities: UniqueMap<UniqueMap<i32>>,
}

impl PackedSnapshot {
    /// Check the field roster that PM-Sync accepts before its global apply.
    /// Unknown plugin fields remain in the Packed snapshot.
    pub fn validate_pm_sync_fields(&self) -> Result<(), RecordsError> {
        for field in REQUIRED {
            if !self.data().contains_key(field) {
                return Err(RecordsError::Missing(field));
            }
        }
        for field in LOCAL_ONLY {
            if self.data().contains_key(field) {
                return Err(RecordsError::LocalField(field));
            }
        }
        Ok(())
    }

    /// Stage the three fields together. This never edits the original Packed data.
    pub fn player_records(
        &self,
        max_field_bytes: usize,
    ) -> Result<HuskPlayerRecords, RecordsError> {
        let read = |key| {
            let field = self.data().get(key).ok_or(RecordsError::Missing(key))?;
            if field.len() > max_field_bytes {
                return Err(RecordsError::SizeLimit(key));
            }
            Ok(field.as_str())
        };
        let data = read(PERSISTENT_DATA)?;
        let (Tag::Compound(persistent_data), used) =
            nbt::parse_snbt(data).map_err(|_| RecordsError::PersistentData)?
        else {
            return Err(RecordsError::PersistentData);
        };
        if !data[used..].chars().all(|c| nbt::snbt_whitespace(c as u32)) {
            return Err(RecordsError::PersistentData);
        }
        let progress: Vec<Advancement> = serde_json::from_str(read(ADVANCEMENTS)?)
            .map_err(|error| RecordsError::Json(ADVANCEMENTS, error))?;
        let mut advancements = BTreeMap::new();
        for entry in progress {
            let key =
                Identifier::parse_namespaced(&entry.key).ok_or(RecordsError::AdvancementKey)?;
            if advancements
                .insert(key, entry.completed_criteria.0)
                .is_some()
            {
                return Err(RecordsError::DuplicateAdvancement);
            }
        }
        let stats: Statistics = serde_json::from_str(read(STATISTICS)?)
            .map_err(|error| RecordsError::Json(STATISTICS, error))?;
        let counters = |map: UniqueMap<UniqueMap<i32>>| {
            map.0
                .into_iter()
                .map(|(key, values)| (key, values.0))
                .collect()
        };
        Ok(HuskPlayerRecords {
            persistent_data,
            advancements,
            statistics: HuskStatistics {
                generic: stats.generic.0,
                blocks: counters(stats.blocks),
                items: counters(stats.items),
                entities: counters(stats.entities),
            },
        })
    }
}
