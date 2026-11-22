use horizon_items::{
    husksync_inventory::{ENDER_CHEST_KEY, HuskPlayerItems, INVENTORY_KEY, InventoryError},
    husksync_packed::{
        BukkitMinecraftVersion, PackedCompression, PackedError, PackedLimits, PackedSnapshot,
    },
};

// Original Bukkit HuskSync 4.0.0 Packed fixture, emitted by the Java oracle
// in tools/fixtures/pm-sync/SnapshotInventoryOracle.java.
const JAVA_SNAPPY: &[u8] = include_bytes!("fixtures/husksync/husk-26.2.snappy");
const JAVA_JSON: &str = include_str!("fixtures/husksync/husk-26.2.json");
const PAPER: BukkitMinecraftVersion = BukkitMinecraftVersion::V26_2;
const HORIZON: BukkitMinecraftVersion = BukkitMinecraftVersion::V26_3;
const LIMITS: PackedLimits = PackedLimits {
    max_payload_bytes: 4096,
    max_json_bytes: 4096,
};

fn plain(json: &str, recipient: BukkitMinecraftVersion) -> Result<PackedSnapshot, PackedError> {
    PackedSnapshot::decode(json.as_bytes(), PackedCompression::Json, recipient, LIMITS)
}

#[test]
fn opens_original_raw_snappy_and_projects_the_real_inventory_fields() {
    let from_java =
        PackedSnapshot::decode(JAVA_SNAPPY, PackedCompression::Snappy, PAPER, LIMITS).unwrap();
    assert_eq!(from_java.source_version(), PAPER);
    assert_eq!(
        from_java.data()[INVENTORY_KEY],
        include_str!("fixtures/husksync/inventory.snbt")
    );
    assert_eq!(
        from_java.data()[ENDER_CHEST_KEY],
        include_str!("fixtures/husksync/ender-chest.snbt")
    );
    let fields = from_java.inventory(LIMITS.max_json_bytes).unwrap();
    assert_eq!(fields.source_data_version(), PAPER.data_version());
    assert_eq!(
        fields
            .encode(PAPER.data_version(), LIMITS.max_json_bytes)
            .unwrap()
            .inventory,
        from_java.data()[INVENTORY_KEY]
    );

    let json = from_java
        .encode(PackedCompression::Json, PAPER, LIMITS)
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&json).unwrap(),
        serde_json::from_str::<serde_json::Value>(JAVA_JSON).unwrap(),
    );
    let rust = plain(JAVA_JSON, HORIZON)
        .unwrap()
        .encode(PackedCompression::Snappy, PAPER, LIMITS)
        .unwrap();
    assert_eq!(
        snap::raw::Decoder::new().decompress_vec(&rust).unwrap(),
        json
    );
    assert_eq!(
        PackedSnapshot::decode(&rust, PackedCompression::Snappy, HORIZON, LIMITS)
            .unwrap()
            .data(),
        from_java.data(),
    );
}

#[test]
fn retains_unknown_fields_and_disabled_feature_presence_when_replacing_both_inventories() {
    let extension =
        r#"{"未识别":"<>&","number":184467440737095516161234567890,"literal":"\u0026"}"#;
    let input = JAVA_JSON.replacen('{', &format!("{{\"addon:metadata\":{extension},"), 1).replacen(
        r#""data":{"#,
        r#""data":{"addon:payload":"旧数据\u0000🙂","husksync:location":"{}","husksync:flight_status":"{}","#,
        1,
    );
    let mut packed = plain(&input, PAPER).unwrap();
    let replacement = HuskPlayerItems::parse(
        Some(include_str!("fixtures/husksync/empty-inventory.snbt")),
        Some(include_str!("fixtures/husksync/empty-ender-chest.snbt")),
        PAPER.data_version(),
        LIMITS.max_json_bytes,
    )
    .unwrap();
    assert!(matches!(
        packed.replace_inventory(&replacement, 1),
        Err(PackedError::Inventory(InventoryError::SizeLimit))
    ));
    assert_eq!(
        packed.data()[INVENTORY_KEY],
        include_str!("fixtures/husksync/inventory.snbt")
    );
    packed
        .replace_inventory(&replacement, LIMITS.max_json_bytes)
        .unwrap();
    let bytes = packed
        .encode(PackedCompression::Snappy, PAPER, LIMITS)
        .unwrap();
    let restored =
        PackedSnapshot::decode(&bytes, PackedCompression::Snappy, PAPER, LIMITS).unwrap();
    assert_eq!(restored.metadata()["addon:metadata"].get(), extension);
    assert_eq!(restored.metadata()["server_name"].get(), "\"fixture\"");
    assert_eq!(restored.data()["addon:payload"], "旧数据\0🙂");
    assert_eq!(restored.data()["husksync:location"], "{}");
    assert_eq!(restored.data()["husksync:flight_status"], "{}");
    assert!(!restored.data().contains_key("husksync:game_mode"));
    assert_eq!(restored.data()[ENDER_CHEST_KEY], "{size:27}");
}

#[test]
fn refuses_newer_source_and_native_items_under_an_older_label() {
    let newer = JAVA_JSON.replace(
        r#""minecraft_version":"26.2""#,
        r#""minecraft_version":"26.3""#,
    );
    assert!(matches!(
        plain(&newer, PAPER),
        Err(PackedError::MinecraftVersion)
    ));
    let newer = plain(&newer, HORIZON).unwrap();
    assert!(matches!(
        newer.encode(PackedCompression::Snappy, PAPER, LIMITS),
        Err(PackedError::MinecraftVersion)
    ));
    let mut old = plain(JAVA_JSON, PAPER).unwrap();
    let result = old.replace_inventory(
        &newer.inventory(LIMITS.max_json_bytes).unwrap(),
        LIMITS.max_json_bytes,
    );
    assert!(matches!(result, Err(PackedError::MinecraftVersion)));
    let unknown = JAVA_JSON.replace(
        r#""minecraft_version":"26.2""#,
        r#""minecraft_version":"27.0""#,
    );
    assert!(matches!(
        plain(&unknown, HORIZON),
        Err(PackedError::MinecraftVersion)
    ));
    let fabric = JAVA_JSON.replace(r#""platform_type":"bukkit""#, r#""platform_type":"fabric""#);
    assert!(matches!(
        plain(&fabric, HORIZON),
        Err(PackedError::Platform)
    ));
    let legacy = JAVA_JSON.replace(r#""format_version":5"#, r#""format_version":4"#);
    assert!(matches!(
        plain(&legacy, HORIZON),
        Err(PackedError::FormatVersion)
    ));
}

#[test]
fn refuses_oversize_snappy_and_malformed_or_wrong_adapters() {
    let limit = PackedLimits {
        max_payload_bytes: JAVA_SNAPPY.len(),
        max_json_bytes: JAVA_JSON.len() - 1,
    };
    assert!(matches!(
        PackedSnapshot::decode(JAVA_SNAPPY, PackedCompression::Snappy, PAPER, limit),
        Err(PackedError::SizeLimit)
    ));
    let oversized_header = [0xff, 0xff, 0xff, 0xff, 0x07];
    assert!(matches!(
        PackedSnapshot::decode(&oversized_header, PackedCompression::Snappy, PAPER, LIMITS),
        Err(PackedError::SizeLimit)
    ));
    let limit = PackedLimits {
        max_payload_bytes: JAVA_SNAPPY.len() - 1,
        ..LIMITS
    };
    assert!(matches!(
        PackedSnapshot::decode(JAVA_SNAPPY, PackedCompression::Snappy, PAPER, limit),
        Err(PackedError::SizeLimit)
    ));
    let mut trailing = JAVA_SNAPPY.to_vec();
    trailing.push(0);
    assert!(matches!(
        PackedSnapshot::decode(&trailing, PackedCompression::Snappy, PAPER, LIMITS),
        Err(PackedError::Snappy(_))
    ));
    assert!(
        PackedSnapshot::decode(
            JAVA_JSON.as_bytes(),
            PackedCompression::Snappy,
            PAPER,
            LIMITS
        )
        .is_err()
    );
    assert!(matches!(
        PackedSnapshot::decode(JAVA_SNAPPY, PackedCompression::Json, PAPER, LIMITS),
        Err(PackedError::Json(_))
    ));
    let snapshot = plain(JAVA_JSON, PAPER).unwrap();
    for mode in [PackedCompression::Json, PackedCompression::Snappy] {
        let max_payload_bytes = snapshot.encode(mode, PAPER, LIMITS).unwrap().len() - 1;
        let tight = PackedLimits {
            max_payload_bytes,
            ..LIMITS
        };
        assert!(matches!(
            snapshot.encode(mode, PAPER, tight),
            Err(PackedError::SizeLimit)
        ));
    }
}

#[test]
fn refuses_duplicate_or_unusable_json_fields_and_distinguishes_absent_inventory() {
    let duplicate_root = JAVA_JSON.replacen('{', r#"{"format_version":5,"#, 1);
    let duplicate_data =
        JAVA_JSON.replacen(r#""data":{"#, r#""data":{"husksync:inventory":"{}","#, 1);
    let number_data = JAVA_JSON.replacen(r#""data":{"#, r#""data":{"addon:payload":1,"#, 1);
    for input in [duplicate_root, duplicate_data, number_data] {
        assert!(matches!(plain(&input, PAPER), Err(PackedError::Json(_))));
    }
    let missing = JAVA_JSON.replace("husksync:ender_chest", "addon:ender_chest");
    let missing = plain(&missing, PAPER).unwrap();
    assert!(matches!(
        missing.inventory(LIMITS.max_json_bytes),
        Err(InventoryError::MissingField(ENDER_CHEST_KEY))
    ));
}
