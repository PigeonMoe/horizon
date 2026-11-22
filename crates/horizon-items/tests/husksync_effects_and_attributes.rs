use horizon_items::{
    husksync_effects_and_attributes::{ATTRIBUTES_KEY, EffectsAttributesError, POTION_EFFECTS_KEY},
    husksync_packed::{BukkitMinecraftVersion, PackedCompression, PackedLimits, PackedSnapshot},
};
use horizon_world::attributes::Operation;
use serde_json::Value;

const JAVA: &str = include_str!("fixtures/husksync/husk-effects-attributes-26.2.json");
const PAPER: BukkitMinecraftVersion = BukkitMinecraftVersion::V26_2;
const LIMITS: PackedLimits = PackedLimits {
    max_payload_bytes: 8192,
    max_json_bytes: 8192,
};

fn open(json: &str) -> PackedSnapshot {
    PackedSnapshot::decode(json.as_bytes(), PackedCompression::Json, PAPER, LIMITS).unwrap()
}

fn edit_field(key: &str, replacement: Option<&str>) -> PackedSnapshot {
    let mut document: Value = serde_json::from_str(JAVA).unwrap();
    let data = document["data"].as_object_mut().unwrap();
    if let Some(raw) = replacement {
        data.insert(key.into(), Value::String(raw.into()));
    } else {
        data.remove(key);
    }
    open(&document.to_string())
}

#[test]
fn stages_the_original_paper_effect_and_attribute_fields() {
    let packed = open(JAVA);
    let staged = packed.startup_effects_and_attributes().unwrap();
    assert_eq!(packed.effects().unwrap(), staged.effects);
    assert_eq!(packed.attributes().unwrap(), staged.attributes);
    let [speed, regeneration] = staged.effects.as_slice() else {
        panic!("expected two effects after the Java ambient filter");
    };
    assert_eq!(
        (speed.effect_type.as_str(), speed.duration, speed.amplifier),
        ("minecraft:speed", -1, 255)
    );
    assert_eq!(
        (speed.is_ambient, speed.show_particles, speed.has_icon),
        (false, false, true)
    );
    assert_eq!(
        (
            regeneration.effect_type.as_str(),
            regeneration.duration,
            regeneration.amplifier
        ),
        ("minecraft:regeneration", 73, 0)
    );
    assert_eq!(
        (
            regeneration.is_ambient,
            regeneration.show_particles,
            regeneration.has_icon
        ),
        (false, true, false)
    );

    let [health, damage, speed] = staged.attributes.as_slice() else {
        panic!("expected three Paper attribute records");
    };
    assert_eq!(health.name, "minecraft:max_health");
    assert_eq!(health.base_value, 1200.125);
    assert_eq!(health.modifiers.len(), 1);
    assert_eq!(health.modifiers[0].name, "oracle:health");
    assert_eq!(
        health.modifiers[0].native_operation(),
        Some(Operation::AddValue)
    );
    assert_eq!(health.modifiers[0].equipment_slot_group, "any");
    assert_eq!(damage.name, "minecraft:attack_damage");
    assert_eq!(
        damage.modifiers[0].native_operation(),
        Some(Operation::AddMultipliedBase)
    );
    assert_eq!(speed.name, "minecraft:movement_speed");
    assert_eq!(speed.base_value.to_bits(), 0.12012345678912345f64.to_bits());
    assert_eq!(
        speed.modifiers[0].native_operation(),
        Some(Operation::AddMultipliedTotal)
    );
    assert_eq!(speed.modifiers[0].amount.to_bits(), (-0.0f64).to_bits());
    assert_eq!(speed.modifiers[0].effective_slot_group(), "mainhand");
    assert_eq!(speed.modifiers[0].equipment_slot, 0);

    let bytes = packed
        .encode(PackedCompression::Snappy, PAPER, LIMITS)
        .unwrap();
    let reopened =
        PackedSnapshot::decode(&bytes, PackedCompression::Snappy, PAPER, LIMITS).unwrap();
    assert_eq!(packed.data(), reopened.data());
}

#[test]
fn preflight_matches_ambient_and_amplifier_normalization_with_original_values_preserved() {
    let raw = r#"[
        {"type":"minecraft:speed","amplifier":300,"duration":-1,"is_ambient":false,"show_particles":false,"has_icon":true,"addon:meta":"原样"},
        {"type":"minecraft:regeneration","amplifier":-4,"duration":-19,"is_ambient":false,"show_particles":true,"has_icon":false},
        {"type":"minecraft:haste","amplifier":9,"duration":72,"is_ambient":true,"show_particles":true,"has_icon":false}
    ]"#;
    let packed = edit_field(POTION_EFFECTS_KEY, Some(raw));
    let effects = packed.effects().unwrap();
    assert_eq!(effects[0].amplifier, 300);
    assert_eq!(effects[1].amplifier, -4);
    assert!(effects[2].is_ambient);
    let staged = packed.startup_effects_and_attributes().unwrap();
    assert_eq!(staged.effects.len(), 2);
    assert_eq!(
        (staged.effects[0].amplifier, staged.effects[0].duration),
        (255, -1)
    );
    assert_eq!(
        (staged.effects[1].amplifier, staged.effects[1].duration),
        (0, -19)
    );
    let encoded = packed
        .encode(PackedCompression::Json, PAPER, LIMITS)
        .unwrap();
    let restored = open(std::str::from_utf8(&encoded).unwrap());
    assert_eq!(restored.data()[POTION_EFFECTS_KEY], raw);
    assert!(!restored.data().contains_key("husksync:game_mode"));
}

#[test]
fn preserves_legacy_slot_metadata_and_applies_the_original_group_lookup() {
    let raw = r#"{"attributes":[{"name":"extension:attribute","baseValue":-2.0,"modifiers":[
        {"name":"oracle:first","amount":-1.25,"operation":0,"equipment_slot":5,"equipment_slot_group":"MaInHaNd","uuid":"8c0e7dd0-005e-48a5-9d49-28afb237bc91"},
        {"name":"oracle:future","amount":2,"operation":1,"equipment_slot_group":"future-group","vendor":{"size":3}},
        {"name":"oracle:default","amount":3,"operation":2}
    ]}]}"#;
    let packed = edit_field(ATTRIBUTES_KEY, Some(raw));
    let snapshot = packed.startup_effects_and_attributes().unwrap();
    let modifiers = &snapshot.attributes[0].modifiers;
    assert_eq!(snapshot.attributes[0].base_value, -2.0);
    assert_eq!(modifiers[0].equipment_slot_group, "MaInHaNd");
    assert_eq!(modifiers[0].equipment_slot, 5);
    assert_eq!(modifiers[0].effective_slot_group(), "mainhand");
    assert!(modifiers[0].uuid.is_some());
    assert_eq!(modifiers[1].equipment_slot_group, "future-group");
    assert_eq!(modifiers[1].effective_slot_group(), "any");
    assert_eq!(modifiers[2].effective_slot_group(), "any");
    assert_eq!(packed.data()[ATTRIBUTES_KEY], raw);
}

#[test]
fn stages_nothing_when_required_data_cannot_be_applied() {
    for key in [POTION_EFFECTS_KEY, ATTRIBUTES_KEY] {
        let missing = edit_field(key, None);
        assert!(
            matches!(missing.startup_effects_and_attributes(), Err(EffectsAttributesError::MissingField(field)) if field == key)
        );
    }
    let empty = edit_field(POTION_EFFECTS_KEY, Some("[]"));
    assert!(
        empty
            .startup_effects_and_attributes()
            .unwrap()
            .effects
            .is_empty()
    );
    let source = open(JAVA);
    let attributes = &source.data()[ATTRIBUTES_KEY];
    let bad = edit_field(
        ATTRIBUTES_KEY,
        Some(&attributes.replacen("\"operation\":0", "\"operation\":3", 1)),
    );
    assert!(bad.attributes().is_ok());
    assert!(matches!(
        bad.startup_effects_and_attributes(),
        Err(EffectsAttributesError::ModifierOperation(3))
    ));
    let derived = edit_field(
        ATTRIBUTES_KEY,
        Some(&attributes.replace("oracle:health", "pmattibute:health-flat")),
    );
    assert!(
        matches!(derived.startup_effects_and_attributes(), Err(EffectsAttributesError::PmaModifier(id)) if id == "pmattibute:health-flat")
    );
    let duplicate = edit_field(
        ATTRIBUTES_KEY,
        Some(&attributes.replace("minecraft:movement_speed", "minecraft:max_health")),
    );
    assert!(matches!(
        duplicate.startup_effects_and_attributes(),
        Err(EffectsAttributesError::DuplicateIdentifier {
            field: "attribute",
            ..
        })
    ));
    let effect = &source.data()[POTION_EFFECTS_KEY];
    let duplicate = edit_field(
        POTION_EFFECTS_KEY,
        Some(&effect.replace("minecraft:regeneration", "minecraft:speed")),
    );
    assert!(matches!(
        duplicate.startup_effects_and_attributes(),
        Err(EffectsAttributesError::DuplicateIdentifier {
            field: "potion effect",
            ..
        })
    ));
    let nonfinite = edit_field(
        ATTRIBUTES_KEY,
        Some(&attributes.replace("1200.125", "1e9999")),
    );
    assert!(matches!(
        nonfinite.startup_effects_and_attributes(),
        Err(EffectsAttributesError::InvalidJson {
            field: ATTRIBUTES_KEY,
            ..
        })
    ));
}
