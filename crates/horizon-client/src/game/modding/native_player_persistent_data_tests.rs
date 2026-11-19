//! `player_persistent_data` and `player_set_persistent_data` against real LAN guests, through the
//! public player handle.

use super::native_guest_turn_tests::with_guests;
use horizon_core::nbt::{Compound, Tag};
use mod_api::{PlayerDataRefusal, prelude::*};

fn saved_data(game: &super::Game<'_>, conn: u32) -> Option<Tag> {
    let hub = &game.lan.as_ref().unwrap().hub;
    hub.guest(conn)?
        .saved
        .as_ref()?
        .get("NeoForgeData")
        .cloned()
}

/// A chain of compounds `levels` deep, each holding the next under `k`.
fn nested(levels: usize) -> Compound {
    let mut data = Compound::new();
    for _ in 1..levels {
        let mut outer = Compound::new();
        outer.insert("k".into(), Tag::Compound(data));
        data = outer;
    }
    data
}

/// The mods' compound goes into the player's own saved record, in their turn or out of it,
/// and an empty one takes the key away.
#[test]
#[ignore = "daily"]
fn persistent_data_is_kept_in_the_players_record() {
    with_guests(|game| {
        let players = game.players().server_players();
        let (a, b) = (players[0], players[1]);
        let radiation = mod_api::Identifier::parse("slimefun:radiation").unwrap();
        let paper_value = Tag::String("27;0".into());
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |guest| {
                guest.saved = Some(Tag::compound([
                    (
                        "BukkitValues",
                        Tag::compound([("slimefun:radiation", paper_value.clone())]),
                    ),
                    ("slimefun:radiation", Tag::Int(99)),
                ]));
            });
        assert_eq!(a.paper_persistent_value(game, &radiation), None);
        assert_eq!(
            b.paper_persistent_value(game, &radiation),
            Some(paper_value.clone())
        );
        game.as_guest(13, |game| {
            assert_eq!(
                b.paper_persistent_value(game, &radiation),
                Some(paper_value)
            );
        })
        .unwrap();
        assert_eq!(b.persistent_data(game), Some(Compound::new()));
        let mut data = Compound::new();
        data.insert("husksync:key".into(), Tag::Int(7));
        assert_eq!(b.set_persistent_data(game, data.clone()), Ok(()));
        assert_eq!(saved_data(game, 13), Some(Tag::Compound(data.clone())));
        assert_eq!(saved_data(game, 12), None);
        game.as_guest(12, |game| {
            assert_eq!(b.persistent_data(game), Some(data.clone()));
            assert_eq!(a.set_persistent_data(game, data.clone()), Ok(()));
            assert_eq!(a.persistent_data(game), Some(data.clone()));
        })
        .unwrap();
        assert_eq!(saved_data(game, 12), Some(Tag::Compound(data)));
        assert_eq!(b.set_persistent_data(game, Compound::new()), Ok(()));
        assert_eq!(saved_data(game, 13), None);
        assert_eq!(b.persistent_data(game), Some(Compound::new()));
    });
}

/// This API refuses with its own names, writing nothing: data 512 levels deep or more in the
/// player's file (511 are written), a dead player, and a handle whose player has left.
#[test]
#[ignore = "daily"]
fn each_persistent_data_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let mut kept = Compound::new();
        kept.insert("kept".into(), Tag::Byte(1));
        assert_eq!(b.set_persistent_data(game, kept.clone()), Ok(()));
        assert_eq!(b.set_persistent_data(game, nested(511)), Ok(()));
        assert_eq!(b.persistent_data(game), Some(nested(511)));
        assert_eq!(b.set_persistent_data(game, kept.clone()), Ok(()));
        assert_eq!(
            b.set_persistent_data(game, nested(512)),
            Err(PlayerDataRefusal::TooDeep)
        );
        assert_eq!(
            b.persistent_data(game),
            Some(kept.clone()),
            "nothing was written"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Dead)
        );
        assert_eq!(
            b.persistent_data(game),
            Some(kept),
            "a dead player is read, not written"
        );
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = false);
        let index = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .connection_index(13)
            .unwrap();
        game.lan.as_mut().unwrap().hub.guests.remove(index);
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Stale)
        );
        assert_eq!(b.persistent_data(game), None);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (client, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(client, "PlayerDataRefusalTest").unwrap();
        assert_eq!(
            b.set_persistent_data(game, Compound::new()),
            Err(PlayerDataRefusal::Client)
        );
    });
}

/// A death respawn carries only `PlayerPersisted` of the mods' compound: a key outside it is
/// lost, a key inside it is read back.
#[test]
#[ignore = "daily"]
fn a_respawn_keeps_only_player_persisted() {
    with_guests(|game| {
        let b = game.players().server_players()[1];
        let mut inside = Compound::new();
        inside.insert("inside".into(), Tag::Int(2));
        let mut data = Compound::new();
        data.insert("outside".into(), Tag::Int(1));
        data.insert("PlayerPersisted".into(), Tag::Compound(inside.clone()));
        assert_eq!(b.set_persistent_data(game, data.clone()), Ok(()));
        assert_eq!(b.persistent_data(game), Some(data), "kept while alive");
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |g| g.dead = true);
        game.guest_respawn(13);
        let mut after = Compound::new();
        after.insert("PlayerPersisted".into(), Tag::Compound(inside));
        assert_eq!(b.persistent_data(game), Some(after));
        let outside = b
            .persistent_data(game)
            .unwrap()
            .get(&"outside".into())
            .cloned();
        assert_eq!(outside, None, "a key outside PlayerPersisted is lost");
    });
}

#[test]
#[ignore = "daily"]
fn saved_flags_are_scoped_and_preserve_the_live_and_archived_donor_distinction() {
    use horizon_client::{lan::playerdata, save::write_dat};
    use mod_api::{
        Identifier, PlayerFlagReadError as Error,
        content::crash::{ModCall, enter_native_mod},
    };
    use std::fs;

    fn record(uuid: u128, entries: impl IntoIterator<Item = (&'static str, Tag)>) -> Tag {
        Tag::compound([
            ("UUID", Tag::IntArray(playerdata::uuid_ints(uuid).to_vec())),
            (
                "NeoForgeData",
                Tag::compound([("PlayerPersisted", Tag::compound(entries))]),
            ),
        ])
    }

    with_guests(|game| {
        game.world_root = game.save.root.clone();
        let online = game.players().server_players()[1].uuid();
        let offline = 0x7ab8b15dd1204725ab3c992723100f4b;
        let unknown = 0xbeef;
        let path = playerdata::path(&game.world_root, offline);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let flag = |key: &str| Identifier::parse(key).unwrap();
        let keys = [
            flag("slimefun:one"),
            flag("slimefun:two"),
            flag("slimefun:wrong_type"),
        ];

        let host: &dyn mod_api::ServerHost = game;
        assert_eq!(
            host.player_saved_true_flags(unknown, &keys),
            Err(Error::NotOwned)
        );
        let _scope = enter_native_mod(ModCall {
            id: "slimefun",
            hook: "saved_flags_test",
            part: None,
        });
        let host: &dyn mod_api::ServerHost = game;
        assert_eq!(host.player_saved_true_flags(unknown, &keys), Ok(None));
        assert_eq!(
            host.player_saved_true_flags(offline, &[flag("other:private")]),
            Err(Error::NotOwned)
        );
        assert_eq!(
            host.player_saved_true_flags(offline, &[]),
            Err(Error::KeyCount)
        );

        write_dat(
            &path,
            record(
                offline,
                [
                    ("slimefun:two", Tag::Byte(1)),
                    ("slimefun:wrong_type", Tag::Int(1)),
                    ("other:private", Tag::Byte(1)),
                ],
            ),
        )
        .unwrap();
        write_dat(
            &path.with_extension("dat_old"),
            record(offline, [("slimefun:one", Tag::Byte(1))]),
        )
        .unwrap();
        write_dat(
            &playerdata::path(&game.world_root, online).with_extension("dat_old"),
            record(online, [("slimefun:two", Tag::Byte(1))]),
        )
        .unwrap();
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update_connection(13, |guest| {
                guest.saved = Some(record(
                    online,
                    [
                        ("slimefun:one", Tag::Byte(1)),
                        ("slimefun:two", Tag::Byte(0)),
                    ],
                ));
            })
            .unwrap();

        let host: &dyn mod_api::ServerHost = game;
        assert_eq!(
            host.player_saved_true_flags(online, &keys),
            Ok(Some(vec![keys[0].clone()]))
        );
        assert_eq!(
            host.player_saved_true_flags(offline, &keys),
            Ok(Some(vec![keys[1].clone()]))
        );
        let before = fs::read(&path).unwrap();
        assert_eq!(
            host.player_saved_true_flags(offline, &[keys[0].clone()]),
            Ok(Some(vec![]))
        );
        assert_eq!(fs::read(&path).unwrap(), before);

        fs::write(&path, b"damaged").unwrap();
        assert_eq!(
            host.player_saved_true_flags(offline, &keys),
            Ok(Some(vec![keys[0].clone()]))
        );
        fs::remove_file(path.with_extension("dat_old")).unwrap();
        assert_eq!(
            host.player_saved_true_flags(offline, &keys),
            Err(Error::Corrupt)
        );

        // A copied or symlinked file cannot supply a different profile's flags.
        write_dat(&path, record(unknown, [("slimefun:one", Tag::Byte(1))])).unwrap();
        assert_eq!(
            host.player_saved_true_flags(offline, &keys),
            Err(Error::Corrupt)
        );
        #[cfg(unix)]
        {
            fs::remove_file(&path).unwrap();
            let hidden = path.with_extension("private");
            write_dat(&hidden, record(offline, [("slimefun:one", Tag::Byte(1))])).unwrap();
            std::os::unix::fs::symlink(&hidden, &path).unwrap();
            assert_eq!(
                host.player_saved_true_flags(offline, &keys),
                Err(Error::Corrupt)
            );
        }
    });
}
