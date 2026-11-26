//! Required plugin data carried by a version 2 hand-over record.

use horizon_core::identifier::Identifier;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const RECORD_VERSION: i64 = 2;
const MAX_SECTIONS: usize = 16;
const MAX_ID_BYTES: usize = 128;
// The control socket permits a 16 MiB request line. This allows an 8 MiB opaque
// profile, with up to about 10.67 MiB of base64 plus the native record and headers.
pub(super) const MAX_PROFILE_BYTES: usize = 8 << 20;

pub(super) struct Section {
    pub id: Identifier,
    pub version: u32,
    pub payload: Vec<u8>,
}

fn section_id(raw: &str) -> Option<Identifier> {
    (raw.len() <= MAX_ID_BYTES)
        .then(|| Identifier::parse_namespaced(raw))
        .flatten()
}

pub(super) fn parse_roster(text: &str) -> Result<BTreeMap<Identifier, u32>, String> {
    let mut roster = BTreeMap::new();
    for pair in text.split(';') {
        let (id, schema) = pair
            .split_once('@')
            .ok_or("required_profiles needs id@schema entries")?;
        let id = section_id(id).ok_or("required_profiles needs fully namespaced IDs")?;
        let schema = schema
            .parse::<u32>()
            .ok()
            .filter(|version| *version > 0)
            .ok_or("required_profiles needs positive schema versions")?;
        if roster.insert(id, schema).is_some() {
            return Err("required_profiles contains a duplicate section".into());
        }
        if roster.len() > MAX_SECTIONS {
            return Err("required_profiles may contain up to 16 sections".into());
        }
    }
    Ok(roster)
}

/// All sections in a version 2 record are required. A receiver needs a durable
/// placement participant for every one. Until those participants are available,
/// refuse the import before touching the held guest or acknowledging a retry.
pub(super) fn check_import(record: &Value) -> Result<(), String> {
    let sections = read_required(record)?;
    if sections.is_empty() {
        return Ok(());
    }
    let required: Vec<_> = sections
        .iter()
        .map(|section| format!("{} (schema {})", section.id, section.version))
        .collect();
    Err(format!(
        "no durable receiver is available for required profiles: {}",
        required.join(", ")
    ))
}

/// Decode every required section with one aggregate allocation budget.
pub(super) fn read_required(record: &Value) -> Result<Vec<Section>, String> {
    let version = record.get("version").and_then(Value::as_i64);
    let profiles = record.get("profiles");
    if version != Some(RECORD_VERSION) {
        return if profiles.is_some() {
            Err("required profiles need a version 2 hand-over record".into())
        } else {
            Ok(Vec::new())
        };
    }
    let sections = profiles
        .and_then(Value::as_array)
        .filter(|sections| (1..=MAX_SECTIONS).contains(&sections.len()))
        .ok_or("a version 2 record needs 1 to 16 required profile sections")?;

    let mut ids = BTreeSet::new();
    let mut remaining = MAX_PROFILE_BYTES;
    let mut required = Vec::with_capacity(sections.len());
    for section in sections {
        let id = section["id"]
            .as_str()
            .and_then(section_id)
            .ok_or("a required profile needs a namespaced id of at most 128 bytes")?;
        if !ids.insert(id.clone()) {
            return Err(format!("the required profile {id} is listed twice"));
        }
        let version = section["version"]
            .as_u64()
            .and_then(|number| u32::try_from(number).ok())
            .filter(|number| *number > 0)
            .ok_or_else(|| format!("the required profile {id} needs a positive schema version"))?;
        let encoded = section["payload"]
            .as_str()
            .filter(|payload| !payload.is_empty() && payload.len().is_multiple_of(4))
            .ok_or_else(|| {
                format!("the required profile {id} needs a nonempty padded base64 payload")
            })?;
        if encoded.len() > remaining.div_ceil(3) * 4 {
            return Err("the required profiles exceed 8 MiB in total".into());
        }
        let bytes = crate::skins::base64_decode(encoded)
            .filter(|payload| !payload.is_empty())
            .ok_or_else(|| format!("the required profile {id} has an invalid base64 payload"))?;
        remaining = remaining
            .checked_sub(bytes.len())
            .ok_or("the required profiles exceed 8 MiB in total")?;
        required.push(Section {
            id,
            version,
            payload: bytes,
        });
    }
    Ok(required)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn section(id: &str, payload: &str) -> Value {
        json!({"id": id, "version": 1, "payload": payload})
    }

    #[test]
    fn required_profile_ids_and_schema_must_be_unambiguous() {
        let valid = section("slimefun:profile", "U0ZQMQ==");
        let duplicate = json!({"version": 2, "profiles": [valid, valid]});
        assert!(
            check_import(&duplicate)
                .unwrap_err()
                .contains("listed twice")
        );
        let unqualified = json!({"version": 2, "profiles": [section("profile", "U0ZQMQ==")]});
        assert!(
            check_import(&unqualified)
                .unwrap_err()
                .contains("namespaced id")
        );
        let mut bad_schema = valid.clone();
        bad_schema["version"] = json!(0);
        let bad_schema = json!({"version": 2, "profiles": [bad_schema]});
        assert!(
            check_import(&bad_schema)
                .unwrap_err()
                .contains("positive schema version")
        );
    }

    #[test]
    fn required_profiles_obey_the_aggregate_decoded_budget() {
        let first = super::super::base64(&vec![0; MAX_PROFILE_BYTES - 1]);
        let mut record = json!({"version": 2, "profiles": [
            section("slimefun:profile", &first),
            section("other:profile", "AA==")
        ]});
        assert!(
            check_import(&record)
                .unwrap_err()
                .contains("no durable receiver")
        );
        // One and two bytes have the same base64 string length. The total must
        // be bounded by decoded size as well as encoded allocation.
        record["profiles"][1]["payload"] = json!("AAA=");
        assert!(check_import(&record).unwrap_err().contains("exceed 8 MiB"));
        record["profiles"][1]["payload"] = json!("%A==");
        assert!(
            check_import(&record)
                .unwrap_err()
                .contains("invalid base64")
        );
    }
}
