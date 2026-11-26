use horizon_core::{nbt, version::DATA_VERSION};
use horizon_items::{
    husksync_inventory::{
        ENDER_CHEST_KEY, HuskPlayerItems, INVENTORY_KEY, InventoryError, InventoryFields,
    },
    item_nbt::item_to_nbt_with_items,
    items::{ItemInfo, Items},
};

// Emitted by BukkitSerializer.Inventory/EnderChest from the original HuskSync
// 4.0.0 at c8ab8393868e59ad6f126c34f3f3f8b83ba4feb1 on Paper 26.2 build 132.
// The stock Java decoder also checked the resulting ItemStacks and held slot.
const JAVA_INVENTORY: &str = include_str!("fixtures/husksync/inventory.snbt");
const JAVA_ENDER: &str = include_str!("fixtures/husksync/ender-chest.snbt");
const JAVA_EMPTY_INVENTORY: &str = include_str!("fixtures/husksync/empty-inventory.snbt");
const JAVA_EMPTY_ENDER: &str = include_str!("fixtures/husksync/empty-ender-chest.snbt");
const PAPER_26_2: i32 = 4903;
const LIMIT: usize = 4096;

fn item_registry() -> Items {
    let mut items = Items::empty();
    for id in [
        "minecraft:diamond",
        "minecraft:torch",
        "minecraft:iron_boots",
        "minecraft:iron_helmet",
        "minecraft:shield",
        "minecraft:gold_ingot",
        "minecraft:stone",
    ] {
        items.items.insert(
            id.into(),
            ItemInfo {
                id: id.into(),
                max_stack: 64,
                ..ItemInfo::default()
            },
        );
    }
    items
}

#[test]
fn original_paper_fields_round_trip_and_stage_every_native_slot() {
    let staged =
        HuskPlayerItems::parse(Some(JAVA_INVENTORY), Some(JAVA_ENDER), PAPER_26_2, LIMIT).unwrap();
    assert_eq!(staged.source_data_version(), PAPER_26_2);
    assert_eq!(
        staged.encode(PAPER_26_2, LIMIT).unwrap(),
        InventoryFields {
            inventory: JAVA_INVENTORY.into(),
            ender_chest: JAVA_ENDER.into(),
        }
    );

    let items = item_registry();
    let native = staged.to_native(&items).unwrap();
    assert_eq!(native.held_item_slot, 4);
    let inventory: Vec<_> = native
        .inventory
        .iter()
        .enumerate()
        .filter_map(|(i, stack)| {
            stack
                .as_ref()
                .map(|stack| (i, stack.item.as_ref(), stack.count))
        })
        .collect();
    assert_eq!(
        inventory,
        [
            (4, "minecraft:diamond", 3),
            (32, "minecraft:torch", 17),
            (36, "minecraft:iron_boots", 1),
            (39, "minecraft:iron_helmet", 1),
            (40, "minecraft:shield", 1),
        ]
    );
    assert!(native.ender_chest[..26].iter().all(Option::is_none));
    let ender = native.ender_chest[26].as_ref().unwrap();
    assert_eq!(
        (ender.item.as_ref(), ender.count),
        ("minecraft:gold_ingot", 11)
    );
    let diamond = item_to_nbt_with_items(native.inventory[4].as_ref().unwrap(), &items);
    let source = nbt::parse_snbt(JAVA_INVENTORY).unwrap().0;
    let Some(nbt::Tag::List(source_slots)) = source.get("items").unwrap().get("items") else {
        panic!("Java inventory items list");
    };
    assert_eq!(diamond.get("components"), source_slots[0].get("components"));

    let resaved = HuskPlayerItems::from_native(&native, &items).unwrap();
    assert_eq!(resaved.source_data_version(), DATA_VERSION);
    assert_eq!(
        resaved.encode(PAPER_26_2, LIMIT),
        Err(InventoryError::Version)
    );
    let current = resaved.encode(DATA_VERSION, LIMIT).unwrap();
    assert_eq!(
        current.inventory,
        JAVA_INVENTORY.replace("DataVersion:4903", &format!("DataVersion:{DATA_VERSION}"))
    );
    assert_eq!(
        current.ender_chest,
        JAVA_ENDER.replace("DataVersion:4903", &format!("DataVersion:{DATA_VERSION}"))
    );
}

#[test]
fn present_empty_fields_have_capacity_and_missing_fields_fail() {
    let limit = JAVA_EMPTY_INVENTORY.len();
    let snapshot = HuskPlayerItems::parse(
        Some(JAVA_EMPTY_INVENTORY),
        Some(JAVA_EMPTY_ENDER),
        PAPER_26_2,
        limit,
    )
    .unwrap();
    assert_eq!(
        snapshot.encode(PAPER_26_2, limit).unwrap(),
        InventoryFields {
            inventory: JAVA_EMPTY_INVENTORY.into(),
            ender_chest: JAVA_EMPTY_ENDER.into(),
        }
    );
    let native = snapshot.to_native(&item_registry()).unwrap();
    assert_eq!(native.held_item_slot, 0);
    assert!(
        native
            .inventory
            .iter()
            .chain(&native.ender_chest)
            .all(Option::is_none)
    );
    assert_eq!(
        snapshot.encode(PAPER_26_2, limit - 1),
        Err(InventoryError::SizeLimit)
    );
    assert_eq!(
        HuskPlayerItems::parse(
            Some(JAVA_EMPTY_INVENTORY),
            Some(JAVA_EMPTY_ENDER),
            PAPER_26_2,
            limit - 1
        )
        .unwrap_err(),
        InventoryError::SizeLimit
    );
    for (inventory, ender, key) in [
        (None, Some(JAVA_EMPTY_ENDER), INVENTORY_KEY),
        (Some(JAVA_EMPTY_INVENTORY), None, ENDER_CHEST_KEY),
    ] {
        assert_eq!(
            HuskPlayerItems::parse(inventory, ender, PAPER_26_2, limit).unwrap_err(),
            InventoryError::MissingField(key)
        );
    }
}

#[test]
fn malformed_or_unknown_items_do_not_produce_a_partial_projection() {
    for (from, to, error) in [
        ("Slot:40", "Slot:41", InventoryError::InvalidSlot),
        ("Slot:40", "Slot:39", InventoryError::InvalidSlot),
        ("count:3", "count:0", InventoryError::InvalidStack),
        (
            "held_item_slot:4",
            "held_item_slot:9",
            InventoryError::InvalidHeldSlot,
        ),
        (
            "size:41",
            "size:27",
            InventoryError::InvalidField(INVENTORY_KEY),
        ),
        (
            "DataVersion:4903",
            "DataVersion:5023",
            InventoryError::Version,
        ),
    ] {
        let malformed = JAVA_INVENTORY.replacen(from, to, 1);
        let result = HuskPlayerItems::parse(Some(&malformed), Some(JAVA_ENDER), PAPER_26_2, LIMIT);
        assert_eq!(result.unwrap_err(), error);
    }
    let trailing = format!("{JAVA_ENDER} {{size:27}}");
    assert_eq!(
        HuskPlayerItems::parse(Some(JAVA_INVENTORY), Some(&trailing), PAPER_26_2, LIMIT)
            .unwrap_err(),
        InventoryError::InvalidField(ENDER_CHEST_KEY)
    );
    for tail in [
        JAVA_ENDER.replace("minecraft:gold_ingot", "example:unknown"),
        JAVA_ENDER.replace("Slot:26", "Slot:26,unconverted_legacy_data:1"),
    ] {
        let snapshot =
            HuskPlayerItems::parse(Some(JAVA_INVENTORY), Some(&tail), PAPER_26_2, LIMIT).unwrap();
        assert_eq!(
            snapshot.to_native(&item_registry()),
            Err(InventoryError::InvalidStack)
        );
    }
}

#[test]
fn an_empty_newer_source_still_requires_a_compatible_recipient() {
    let empty = HuskPlayerItems::parse(
        Some(JAVA_EMPTY_INVENTORY),
        Some(JAVA_EMPTY_ENDER),
        DATA_VERSION,
        LIMIT,
    )
    .unwrap();
    assert_eq!(
        empty.encode(PAPER_26_2, LIMIT),
        Err(InventoryError::Version)
    );
    assert_eq!(
        HuskPlayerItems::parse(
            Some(JAVA_EMPTY_INVENTORY),
            Some(JAVA_EMPTY_ENDER),
            DATA_VERSION + 1,
            LIMIT
        )
        .unwrap_err(),
        InventoryError::Version
    );
}
