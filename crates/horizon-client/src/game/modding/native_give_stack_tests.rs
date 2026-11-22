//! `player_give_stack` against a real LAN guest and `item_from_snbt` on the public host.

use super::{api_values::TryIntoApi, native_guest_turn_tests::with_guests};
use horizon_core::nbt::parse_snbt;
use mod_api::{Host, ItemStack, PlayerDataRefusal, prelude::*};

fn api_item(game: &super::Game<'_>, snbt: &str) -> Option<ItemStack> {
    let host: &dyn Host = game;
    host.item_from_snbt(snbt)
}

#[test]
#[ignore = "daily"]
fn saved_gzip_items_use_the_world_codec_and_refuse_partial_data() {
    use flate2::{Compression, write::GzEncoder};
    use horizon_core::nbt::{Limits, NamedTag, Tag, write_named};
    use std::io::Write;

    let gzip = |raw: &[u8]| {
        let mut writer = GzEncoder::new(Vec::new(), Compression::fast());
        writer.write_all(raw).unwrap();
        writer.finish().unwrap()
    };
    let current = horizon_core::version::DATA_VERSION;
    let snbt = r#"{id:"minecraft:diamond",count:3,components:{"minecraft:custom_data":{PublicBukkitValues:{"migration_fixture:original":"saved"},typed:[I;1,2,3]},"minecraft:custom_name":{text:"档案",color:"gold"}}}"#;
    let encode = |version: i32, snbt: &str| {
        let (Tag::Compound(mut compound), _) = parse_snbt(snbt).unwrap() else {
            unreachable!()
        };
        compound.insert("DataVersion".into(), Tag::Int(version));
        write_named(
            &NamedTag {
                name: "".into(),
                tag: Tag::Compound(compound),
            },
            Limits::default(),
        )
        .unwrap()
    };

    with_guests(|game| {
        let host: &dyn mod_api::ServerHost = game;
        let raw = encode(current - 10, snbt);
        let valid = gzip(&raw);
        assert_eq!(
            host.item_from_saved_gzip_nbt(valid.clone()),
            api_item(game, snbt)
        );
        let mut compressed_trailing = valid.clone();
        compressed_trailing.push(0);
        let mut raw_trailing = raw;
        raw_trailing.push(0);
        let invalid = [
            valid[..valid.len() - 4].to_vec(),
            compressed_trailing,
            gzip(&raw_trailing),
            gzip(&encode(current + 1, snbt)),
            gzip(&encode(current, "{id:stone,count:100}")),
            gzip(&encode(current, "{id:air,count:1}")),
            gzip(&vec![0; 16 * 1024 * 1024 + 1]),
        ];
        for bytes in invalid {
            assert!(host.item_from_saved_gzip_nbt(bytes).is_none());
        }
    });
}

#[test]
#[ignore = "daily"]
fn saved_gzip_item_export_keeps_count_and_typed_paper_components() {
    use horizon_core::nbt::{Limits, Tag, read_named};
    use std::io::Read;

    let snbt = r#"{id:"minecraft:diamond",count:99,components:{"minecraft:custom_data":{PublicBukkitValues:{"migration_fixture:original":"原始𠮷\u0000描述"},typed:[I;1,2,3]},"minecraft:custom_name":{text:"档案",color:"gold"},"minecraft:lore":["原版说明","第二行"]}}"#;
    with_guests(|game| {
        let host: &dyn mod_api::ServerHost = game;
        let item = api_item(game, snbt).unwrap();
        let compressed = host.item_to_saved_gzip_nbt(&item).unwrap();
        assert_eq!(
            host.item_from_saved_gzip_nbt(compressed.clone()),
            Some(item.clone())
        );

        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut raw)
            .unwrap();
        let (saved, consumed) = read_named(&raw, Limits::default()).unwrap();
        assert_eq!(consumed, raw.len());
        assert!(saved.name.is_empty());
        let Tag::Compound(mut saved) = saved.tag else {
            panic!("encoded stack must have a compound root");
        };
        assert_eq!(
            saved.remove(&"DataVersion".into()),
            Some(Tag::Int(horizon_core::version::DATA_VERSION))
        );
        assert_eq!(Tag::Compound(saved), parse_snbt(snbt).unwrap().0);

        let mut invalid = item;
        invalid.set_count(100).unwrap();
        assert!(host.item_to_saved_gzip_nbt(&invalid).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn saved_paper_components_keep_stack_data_and_reject_partial_payloads() {
    with_guests(|game| {
        let host: &dyn mod_api::ServerHost = game;
        let current = horizon_core::version::DATA_VERSION;
        let components = [
            (
                "minecraft:custom_data",
                r#"{PublicBukkitValues:{"slimefun:slimefun_item":"BASIC_CIRCUIT_BOARD"},typed:[I;1,2,3],loaded:1b}"#,
            ),
            ("minecraft:custom_name", r#"{text:"电路板",color:"yellow"}"#),
            ("!minecraft:repair_cost", "{}"),
        ];
        let owned = |entries: &[(&str, &str)]| {
            entries
                .iter()
                .map(|&(id, tag)| (id.into(), tag.into()))
                .collect()
        };
        let stack = host
            .item_from_saved_components(current - 10, "minecraft:gold_ingot", 4, owned(&components))
            .unwrap();
        let expected = api_item(game, r#"{id:"minecraft:gold_ingot",count:4,components:{"minecraft:custom_data":{PublicBukkitValues:{"slimefun:slimefun_item":"BASIC_CIRCUIT_BOARD"},typed:[I;1,2,3],loaded:1b},"minecraft:custom_name":{text:"电路板",color:"yellow"},"!minecraft:repair_cost":{}}}"#).unwrap();
        assert_eq!(stack, expected);
        for (version, id, count, patch) in [
            (current + 1, "minecraft:stone", 1, &[][..]),
            (-1, "minecraft:stone", 1, &[]),
            (current, "minecraft:stone", 100, &[]),
            (current, "minecraft:unknown_item", 1, &[]),
            (current, "minecraft:air", 1, &[]),
            (
                current,
                "minecraft:stone",
                1,
                &[
                    ("minecraft:custom_data", "{}"),
                    ("minecraft:custom_data", "{duplicate:1}"),
                ],
            ),
            (
                current,
                "minecraft:stone",
                1,
                &[("minecraft:custom_data", "{}{trailing:1}")],
            ),
        ] {
            assert!(
                host.item_from_saved_components(version, id, count, owned(patch))
                    .is_none()
            );
        }
    });
}

/// What a mod reads from SNBT and writes back is the compound `ItemStack.CODEC` started
/// from, through the API's own stack and back.
#[test]
#[ignore = "daily"]
fn an_snbt_item_goes_through_the_api_unchanged() {
    with_guests(|game| {
        for snbt in [
            r#"{id:"minecraft:diamond_sword",count:1,components:{"minecraft:damage":5,"minecraft:enchantments":{"minecraft:sharpness":3},"minecraft:custom_name":"Blade","minecraft:lore":["one","two"],"minecraft:custom_data":{husksync:{v:1b}}}}"#,
            r#"{id:"minecraft:stone",count:64}"#,
            r#"{id:"minecraft:potion",count:1,components:{"minecraft:potion_contents":{potion:"minecraft:strong_healing"}}}"#,
            r#"{id:"minecraft:written_book",count:1,components:{"minecraft:written_book_content":{title:{raw:"Log"},author:"guest13",pages:[{raw:"one"}]}}}"#,
        ] {
            let engine = game
                .item_from_snbt(snbt)
                .unwrap_or_else(|| panic!("{snbt}"));
            let api = api_item(game, snbt).unwrap();
            assert_eq!(
                Some(&api),
                engine.clone().try_into_api().ok().as_ref(),
                "{snbt}"
            );
            let (tag, _) = parse_snbt(snbt).unwrap();
            assert_eq!(
                horizon_items::item_nbt::item_to_nbt_with_items(&engine, game.items()),
                tag,
                "{snbt}"
            );
        }
    });
}

/// This API refuses with None: text that is not one compound, text with anything but
/// whitespace after the compound, text over 2 MiB, nesting past 512 levels, air and an item
/// the world lacks, and a count outside 1 to 99.
#[test]
#[ignore = "daily"]
fn item_from_snbt_refuses_what_is_not_one_item_compound() {
    with_guests(|game| {
        let host: &dyn Host = game;
        let count = |snbt: &str| host.item_from_snbt(snbt).map(|s| s.count());
        assert_eq!(
            count(r#"{id:"stone"}"#),
            Some(1),
            "stone is minecraft:stone"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:99}"#), Some(99));
        assert_eq!(count(r#"{id:"minecraft:stone",count:1}"#), Some(1));
        assert_eq!(
            count(r#"{id:"minecraft:stone",count:100}"#),
            None,
            "a count past 99 is refused, not clamped"
        );
        assert_eq!(count(r#"{id:"minecraft:stone",count:0}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:-1}"#), None);
        assert_eq!(count(r#"{id:"minecraft:stone",count:"5"}"#), None);
        assert_eq!(count("  {id:\"minecraft:stone\"} \n"), Some(1));
        assert_eq!(count("stone"), None, "no compound");
        assert_eq!(count("-"), None, "no item");
        assert_eq!(count(r#"[{id:"minecraft:stone"}]"#), None);
        assert_eq!(
            count(r#"{id:"minecraft:stone"} x"#),
            None,
            "text after the compound"
        );
        assert_eq!(
            count(r#"{id:"minecraft:stone"}{id:"minecraft:stone"}"#),
            None
        );
        assert_eq!(count(r#"{id:"minecraft:air",count:1}"#), None);
        assert_eq!(count(r#"{id:"nomod:thing",count:1}"#), None);
        assert_eq!(count(r#"{count:1}"#), None);
        let item = r#"{id:"minecraft:stone"}"#;
        let padded = |len: usize| format!("{item}{}", " ".repeat(len - item.len()));
        assert_eq!(count(&padded(2 * 1024 * 1024)), Some(1), "2 MiB is read");
        assert_eq!(
            count(&padded(2 * 1024 * 1024 + 1)),
            None,
            "one byte more is not"
        );
        let nested = |levels: usize| {
            format!(
                r#"{{id:"minecraft:stone",components:{{"minecraft:custom_data":{{a:{}0{}}}}}}}"#,
                "[".repeat(levels),
                "]".repeat(levels)
            )
        };
        assert_eq!(count(&nested(8)), Some(1));
        assert_eq!(count(&nested(600)), None, "nested past 512 levels");
    });
}

/// A given stack goes into the inventory where it fits, components and all; the rest is
/// thrown for the player alone, with no pickup delay.
#[test]
#[ignore = "daily"]
fn a_given_stack_fills_the_inventory_then_drops_for_its_player() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = r#"{id:"minecraft:stone",count:10,components:{"minecraft:custom_name":"Rock"}}"#;
        let named = api_item(game, rock).unwrap();
        let engine = game.item_from_snbt(rock).unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| {
                let dirt = Some(horizon_client::items::ItemStack::new("minecraft:dirt", 64));
                g.inventory.iter_mut().for_each(|s| *s = dirt.clone());
                g.inventory[5] = Some(horizon_client::items::ItemStack {
                    count: 60,
                    ..engine.clone()
                });
            })
            .unwrap();
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, named.clone()), Ok(()));
        let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[5].as_ref().map(|s| s.count), Some(64));
        assert_eq!(game.dropped.len(), before + 1);
        let dropped = game.dropped.last().unwrap();
        assert_eq!(dropped.stack.count, 6);
        assert_eq!(dropped.stack.extra, engine.extra);
        assert_eq!(dropped.pickup_delay, 0, "no pickup delay");
        assert_eq!(
            dropped.target,
            Some(horizon_client::lan::playerdata::uuid_ints(13)),
            "only its player picks it up"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.inventory[7] = None)
            .unwrap();
        assert_eq!(b.give_stack(game, named), Ok(()));
        let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
        assert_eq!(g.inventory[7], Some(engine));
        assert_eq!(game.dropped.len(), before + 1);
    });
}

/// This API refuses a dead player with its own name and gives nothing.
#[test]
#[ignore = "daily"]
fn a_dead_player_is_given_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let rock = api_item(game, r#"{id:"minecraft:stone",count:3}"#).unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        let before = game.dropped.len();
        assert_eq!(b.give_stack(game, rock), Err(PlayerDataRefusal::Dead));
        assert_eq!(b.inventory_slots(game), Some(Vec::new()));
        assert_eq!(game.dropped.len(), before);
    });
}
