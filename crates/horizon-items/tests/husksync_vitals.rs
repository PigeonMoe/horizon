use horizon_items::{
    husksync_packed::{BukkitMinecraftVersion, PackedCompression, PackedLimits, PackedSnapshot},
    husksync_vitals::{EXPERIENCE_KEY, HEALTH_KEY, HUNGER_KEY, VitalsError},
};
use serde_json::Value;

// The original HuskSync 4.0.0 field serializers running on Paper 26.2,
// generated and round-tripped by SnapshotInventoryOracle.java.
const JAVA_VITALS: &str = include_str!("fixtures/husksync/husk-vitals-26.2.json");
const PAPER: BukkitMinecraftVersion = BukkitMinecraftVersion::V26_2;
const LIMITS: PackedLimits = PackedLimits {
    max_payload_bytes: 4096,
    max_json_bytes: 4096,
};

fn snapshot(json: &str) -> PackedSnapshot {
    PackedSnapshot::decode(json.as_bytes(), PackedCompression::Json, PAPER, LIMITS).unwrap()
}

fn variant(edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> PackedSnapshot {
    let mut json: Value = serde_json::from_str(JAVA_VITALS).unwrap();
    edit(json["data"].as_object_mut().unwrap());
    snapshot(&json.to_string())
}

#[test]
fn stages_paper_doubles_floats_food_and_experience_without_normalizing_the_packed_fields() {
    let packed = snapshot(JAVA_VITALS);
    let vitals = packed.startup_vitals().unwrap();
    assert_eq!(vitals.health, packed.health().unwrap());
    assert_eq!(
        vitals.health.health.to_bits(),
        73.12345678912345f64.to_bits()
    );
    assert_eq!(vitals.health.health_scale.to_bits(), (-0.0f64).to_bits());
    assert!(vitals.health.is_health_scaled);
    assert_eq!(vitals.hunger, packed.hunger().unwrap());
    assert_eq!(vitals.hunger.food_level, 27);
    assert_eq!(vitals.hunger.saturation.to_bits(), (-0.0f32).to_bits());
    assert_eq!(vitals.hunger.exhaustion.to_bits(), (-3.25f32).to_bits());
    assert_eq!(vitals.experience, packed.experience().unwrap());
    assert_eq!(vitals.experience.total_experience, 123456789);
    assert_eq!(vitals.experience.exp_level, 37);
    assert_eq!(
        vitals.experience.exp_progress.to_bits(),
        0.23456788f32.to_bits()
    );

    let json = packed
        .encode(PackedCompression::Json, PAPER, LIMITS)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&json).unwrap(),
        serde_json::from_str::<Value>(JAVA_VITALS).unwrap()
    );
}

#[test]
fn startup_rejects_incomplete_required_fields_and_experience_that_paper_cannot_fully_apply() {
    for key in [HEALTH_KEY, HUNGER_KEY, EXPERIENCE_KEY] {
        let missing = variant(|data| {
            data.remove(key);
        });
        assert!(
            matches!(missing.startup_vitals(), Err(VitalsError::MissingField(which)) if which == key)
        );
        let incomplete = variant(|data| {
            data.insert(key.to_owned(), Value::String("{}".to_owned()));
        });
        assert!(
            matches!(incomplete.startup_vitals(), Err(VitalsError::InvalidJson { field, .. }) if field == key)
        );
    }
    for (json, member) in [
        (
            r#"{"total_experience":-1,"exp_level":3,"exp_progress":0.5}"#,
            "total_experience",
        ),
        (
            r#"{"total_experience":10,"exp_level":-1,"exp_progress":0.5}"#,
            "exp_level",
        ),
        (
            r#"{"total_experience":10,"exp_level":3,"exp_progress":1.125}"#,
            "exp_progress",
        ),
    ] {
        let packed = variant(|data| {
            data.insert(EXPERIENCE_KEY.to_owned(), Value::String(json.to_owned()));
        });
        assert!(packed.experience().is_ok());
        assert!(
            matches!(packed.startup_vitals(), Err(VitalsError::InvalidValue(field)) if field == member)
        );
        assert_eq!(packed.data()[EXPERIENCE_KEY], json);
    }
    let negative = variant(|data| {
        data.insert(
            HEALTH_KEY.to_owned(),
            Value::String(r#"{"health":-1,"health_scale":2,"is_health_scaled":false}"#.into()),
        );
    });
    assert!(matches!(
        negative.startup_vitals(),
        Err(VitalsError::InvalidValue("health"))
    ));
}

#[test]
fn stages_java_float_rounding_and_preserves_unknown_members_and_disabled_fields() {
    // This value is above a float midpoint but rounds to that midpoint as a
    // double. Gson takes the double path before narrowing to float.
    let experience = r#"{"total_experience":10,"exp_level":3,"exp_progress":0.5000000298023223876953125000000001,"addon:counter":"保留"}"#;
    let packed = variant(|data| {
        data.insert(
            EXPERIENCE_KEY.to_owned(),
            Value::String(experience.to_owned()),
        );
        data.insert(
            "husksync:location".to_owned(),
            Value::String("{}".to_owned()),
        );
        data.insert(
            "extra:opaque".to_owned(),
            Value::String("原样 🙂".to_owned()),
        );
    });
    assert_eq!(
        packed
            .startup_vitals()
            .unwrap()
            .experience
            .exp_progress
            .to_bits(),
        0.5f32.to_bits()
    );
    let payload = packed
        .encode(PackedCompression::Snappy, PAPER, LIMITS)
        .unwrap();
    let restored =
        PackedSnapshot::decode(&payload, PackedCompression::Snappy, PAPER, LIMITS).unwrap();
    assert_eq!(restored.data()[EXPERIENCE_KEY], experience);
    assert_eq!(restored.data()["husksync:location"], "{}");
    assert_eq!(restored.data()["extra:opaque"], "原样 🙂");
    assert!(!restored.data().contains_key("husksync:flight_status"));
}

#[test]
fn refuses_non_finite_runtime_values_after_java_narrowing() {
    let overflow = variant(|data| {
        data.insert(
            HUNGER_KEY.to_owned(),
            Value::String(r#"{"food_level":5,"saturation":3.5e38,"exhaustion":0}"#.into()),
        );
    });
    assert!(matches!(
        overflow.startup_vitals(),
        Err(VitalsError::InvalidJson {
            field: HUNGER_KEY,
            ..
        })
    ));
    let invalid = variant(|data| {
        data.insert(
            HEALTH_KEY.to_owned(),
            Value::String(r#"{"health":1e9999,"health_scale":20,"is_health_scaled":false}"#.into()),
        );
    });
    assert!(matches!(
        invalid.startup_vitals(),
        Err(VitalsError::InvalidJson {
            field: HEALTH_KEY,
            ..
        })
    ));
}
