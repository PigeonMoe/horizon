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
