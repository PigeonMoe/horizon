use horizon_core::{identifier::Identifier, nbt::Tag};
use horizon_items::{
    husksync_packed::{BukkitMinecraftVersion, PackedCompression, PackedLimits, PackedSnapshot},
    husksync_records::RecordsError,
};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/husksync/husk-records-26.2.json");
const LIMITS: PackedLimits = PackedLimits {
    max_payload_bytes: 32 << 10,
    max_json_bytes: 32 << 10,
};

fn packed(raw: &str) -> PackedSnapshot {
    PackedSnapshot::decode(
        raw.as_bytes(),
        PackedCompression::Json,
        BukkitMinecraftVersion::V26_3,
        LIMITS,
    )
    .unwrap()
}

#[test]
fn reads_real_paper_records_without_rewriting_the_snapshot() {
    let original = packed(FIXTURE);
    let records = original.player_records(LIMITS.max_json_bytes).unwrap();
    assert_eq!(
        records.persistent_data.get(&"fixture:long".into()),
        Some(&Tag::Long(987_654_321_123))
    );
    let pdc = records
        .persistent_data
        .get(&"slimefun:archive".into())
        .unwrap();
    assert_eq!(pdc.get("bytes"), Some(&Tag::ByteArray(vec![1, 254])));
    assert_eq!(pdc.get("title"), Some(&Tag::String("原档".into())));
    let advancement = Identifier::parse_namespaced("minecraft:story/mine_stone").unwrap();
    assert_eq!(
        records.advancements[&advancement]["get_stone"],
        1_700_010_000_123
    );
    let custom = Identifier::parse_namespaced("pigeon:record_fixture").unwrap();
    assert_eq!(records.advancements[&custom]["second"], -1234);
    assert_eq!(records.statistics.generic["jump"], 32);
    assert_eq!(records.statistics.blocks["mined"]["stone"], 21);
    assert_eq!(records.statistics.items["used"]["stone_pickaxe"], 8);
    assert_eq!(records.statistics.entities["killed"]["zombie"], 6);
    let encoded = original
        .encode(
            PackedCompression::Json,
            BukkitMinecraftVersion::V26_3,
            LIMITS,
        )
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap(),
        serde_json::from_str::<Value>(FIXTURE).unwrap()
    );
}

#[test]
fn incomplete_or_ambiguous_records_fail_staging() {
    let original = packed(FIXTURE);
    assert!(matches!(
        original.player_records(16),
        Err(RecordsError::SizeLimit("husksync:persistent_data"))
    ));
    let mut json: Value = serde_json::from_str(FIXTURE).unwrap();
    json["data"]["husksync:persistent_data"] = json!("{} trailing");
    assert!(matches!(
        packed(&json.to_string()).player_records(LIMITS.max_json_bytes),
        Err(RecordsError::PersistentData)
    ));
    json["data"]["husksync:persistent_data"] = json!("{}");
    json["data"]["husksync:advancements"] = json!(
        r#"[{"key":"minecraft:story/root","completed_criteria":{"ok":1}},{"key":"minecraft:story/root","completed_criteria":{"ok":2}}]"#
    );
    assert!(matches!(
        packed(&json.to_string()).player_records(LIMITS.max_json_bytes),
        Err(RecordsError::DuplicateAdvancement)
    ));
    json["data"]["husksync:advancements"] = json!("[]");
    json["data"]["husksync:statistics"] = json!(
        r#"{"generic":{},"blocks":{"mined":{"stone":1,"stone":2}},"items":{},"entities":{}}"#
    );
    assert!(matches!(
        packed(&json.to_string()).player_records(LIMITS.max_json_bytes),
        Err(RecordsError::Json("husksync:statistics", _))
    ));
}

#[test]
fn global_roster_requires_shared_state_and_keeps_world_state_local() {
    let mut raw: Value = serde_json::from_str(FIXTURE).unwrap();
    assert!(matches!(
        packed(FIXTURE).validate_pm_sync_fields(),
        Err(RecordsError::Missing("husksync:inventory"))
    ));
    for key in [
        "inventory",
        "ender_chest",
        "health",
        "experience",
        "attributes",
        "hunger",
        "potion_effects",
    ] {
        raw["data"][format!("husksync:{key}")] = json!("placeholder for the roster check");
    }
    raw["data"]["pigeon:other_plugin"] = json!("preserved private data");
    let complete = packed(&raw.to_string());
    assert!(complete.validate_pm_sync_fields().is_ok());
    assert_eq!(
        complete.data()["pigeon:other_plugin"],
        "preserved private data"
    );
    for key in ["location", "game_mode", "flight_status"] {
        let mut invalid = raw.clone();
        let key = format!("husksync:{key}");
        invalid["data"][key.clone()] = json!("this backend decides the local value");
        assert!(
            matches!(packed(&invalid.to_string()).validate_pm_sync_fields(),
            Err(RecordsError::LocalField(field)) if field == key)
        );
    }
}
