use super::*;
use horizon_client::{
    items::ItemStack,
    lan::{
        hub::{HostEvent, TickCtx},
        source_handover::{Handover, Journal},
    },
};
use horizon_core::{identifier::Identifier, nbt::Tag};
use horizon_protocol::{
    PROTOCOL_VERSION,
    buf::Writer,
    connection::Connection,
    crypto::uuid_string,
    packets::{Empty, configuration as cfg, handshake, login, play::Clientbound as Play},
};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::Arc, time::Duration};

const OWNER: u128 = 12;
const NAME: &str = "ResumeQA";
pub(super) const KEY: &[u8] = b"synthetic-return-resume-fixture";
const PROFILE: &str = "example:profile";
const DIMENSION: &str = "minecraft:the_nether";

pub(super) fn forwarded_login(port: u16, complete: bool) -> Result<Connection, String> {
    let mut client = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).unwrap();
    client
        .reader
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    client
        .send(
            &handshake::Serverbound::Intention(handshake::Intention {
                protocol: PROTOCOL_VERSION,
                host: "127.0.0.1".into(),
                port,
                intent: handshake::LOGIN,
            })
            .encode(),
        )
        .unwrap();
    client
        .send(
            &login::Serverbound::Hello(login::ServerHello {
                name: NAME.into(),
                uuid: OWNER,
            })
            .encode(),
        )
        .unwrap();
    // RFC 2104 SHA-256 over the synthetic version, address, UUID, name and empty properties.
    let mut forwarded = Writer::new();
    forwarded.bytes(&[
        0x07, 0x65, 0x2e, 0xb5, 0x97, 0x6d, 0x39, 0x96, 0x9a, 0xdf, 0x50, 0x6b, 0xfb, 0xd2, 0x57,
        0xed, 0x62, 0x15, 0x7e, 0x9d, 0xc8, 0x6b, 0x3f, 0x39, 0x4e, 0x09, 0x03, 0xf7, 0x44, 0xc7,
        0x44, 0xa8,
    ]);
    forwarded.varint(1);
    forwarded.string("127.0.0.1");
    forwarded.uuid(OWNER);
    forwarded.string(NAME);
    forwarded.varint(0);
    loop {
        let (id, body) = client.recv().unwrap();
        match login::Clientbound::decode(id, &body).unwrap() {
            login::Clientbound::CustomQuery(query) => client
                .send(
                    &login::Serverbound::CustomQueryAnswer(login::CustomQueryAnswer {
                        transaction: query.transaction,
                        payload: Some(forwarded.data.clone()),
                    })
                    .encode(),
                )
                .unwrap(),
            login::Clientbound::Compression(packet) => client.set_compression(packet.threshold),
            login::Clientbound::LoginFinished(packet) => {
                assert!(complete, "an ordinary login reached configuration");
                assert_eq!(packet.profile.uuid, OWNER);
                break;
            }
            login::Clientbound::Disconnect(reason) => return Err(reason.reason_json),
            _ => panic!("unexpected resume login packet"),
        }
    }
    client
        .send(&login::Serverbound::LoginAcknowledged(Empty).encode())
        .unwrap();
    loop {
        let (id, body) = client.recv().unwrap();
        match cfg::Clientbound::decode(id, &body).unwrap() {
            cfg::Clientbound::SelectKnownPacks(packs) => client
                .send(&cfg::Serverbound::SelectKnownPacks(packs).encode())
                .unwrap(),
            cfg::Clientbound::FinishConfiguration(_) => {
                client
                    .send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .unwrap();
                return Ok(client);
            }
            cfg::Clientbound::Disconnect(reason) => {
                panic!("return failed in configuration: {reason:?}")
            }
            _ => {}
        }
    }
}

pub(super) fn wait_for_join(game: &mut Game<'_>) -> u32 {
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        let mut ctx = TickCtx {
            sea_level: 63,
            world: &mut game.world,
            entities: &[],
            next_entity: &mut game.next_entity,
            spawn: [0.5, 64.0, 0.5],
            spawn_angle: [0.0; 2],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &game.border,
            streaming: None,
        };
        if let Some(conn) = game
            .lan
            .as_mut()
            .unwrap()
            .hub
            .poll(&mut ctx)
            .into_iter()
            .find_map(|event| {
                if let HostEvent::Joined(conn) = event {
                    Some(conn)
                } else {
                    None
                }
            })
        {
            return conn;
        }
        assert!(Instant::now() < until, "forwarded return did not arrive");
        std::thread::yield_now();
    }
}

pub(super) fn read_held_view(client: &mut Connection) -> i32 {
    let (mut login, mut blank, mut blinded) = (false, false, None);
    while !login || !blank || blinded.is_none() {
        let (id, body) = client.recv().unwrap();
        match Play::decode(id, &body) {
            Ok(Play::Login(packet)) => {
                assert_eq!(packet.spawn.dimension, DIMENSION);
                login = true;
            }
            Ok(Play::UpdateMobEffect(packet)) if packet.duration == -1 => {
                blinded = Some(packet.effect);
            }
            Ok(Play::ContainerSetContent(packet)) if packet.container == 0 => {
                assert!(packet.items.iter().all(Option::is_none));
                assert!(packet.carried.is_none());
                blank = true;
            }
            _ => {}
        }
    }
    blinded.unwrap()
}

pub(super) fn read_resumed_view(client: &mut Connection, blindness: i32) {
    let (mut unblinded, mut inventory) = (false, false);
    while !unblinded || !inventory {
        let (id, body) = client.recv().unwrap();
        match Play::decode(id, &body) {
            Ok(Play::RemoveMobEffect(packet)) if packet.effect == blindness => unblinded = true,
            Ok(Play::ContainerSetContent(packet)) if packet.container == 0 => {
                let items: Vec<_> = packet.items.iter().flatten().collect();
                if !items.is_empty() {
                    assert!(unblinded);
                    assert_eq!(items.len(), 1);
                    assert_eq!(items[0].count, 3);
                    inventory = true;
                }
            }
            _ => {}
        }
    }
}

fn phase_settings() -> arrivals::Settings {
    arrivals::Settings {
        backend_id: Some("west".into()),
        allowed_sources: ["east".into()].into(),
        required_profiles: [(Identifier::parse_namespaced(PROFILE).unwrap(), 1)].into(),
        ..arrivals::Settings::default()
    }
}

fn seed_committed_return(world: &Path, game: &mut Game<'_>) -> (ReturnAuthorization, Value) {
    let old = Handover {
        owner: OWNER,
        id: 41,
        destination: "old-hop".into(),
        record: b"previous record".to_vec(),
        local_checkpoint: b"previous local checkpoint".to_vec(),
    };
    let journal = Journal::open(world).unwrap();
    journal
        .prepare_versioned(&old, 7.try_into().unwrap())
        .unwrap();
    journal
        .retire(OWNER, old.id, &old.destination, old.record_digest())
        .unwrap();
    let request = json!({
        "record": {
            "version": 2,
            "uuid": uuid_string(OWNER),
            "handover": arrivals::handover_text(53),
            "source": "east",
            "destination": "west",
            "authority_revision": 9,
        },
        "place": false,
    });
    let authorization = ReturnAuthorization {
        retired: journal.retired_anchor(OWNER).unwrap().unwrap(),
        handover: 53,
        revision: 9.try_into().unwrap(),
        source: "east".into(),
        destination: "west".into(),
        record_sha256: receipt::record_digest(&request["record"]).unwrap(),
        request_sha256: receipt::record_digest(&request).unwrap(),
    };
    let received = receipt::Receipt::new(
        receipt::Binding::from_import(&phase_settings(), OWNER, &request).unwrap(),
        false,
        false,
    );
    let payload = b"committed plugin profile";
    let profile = profile_file(world, &authorization);
    std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
    std::fs::write(&profile, payload).unwrap();
    let host = game.lan.as_mut().unwrap();
    host.hub
        .update_guest(12, |guest| {
            guest.saved = Some(Tag::compound([(
                "HorizonTransferProfiles",
                Tag::compound([(
                    PROFILE,
                    Tag::compound([
                        (
                            "generation",
                            Tag::ByteArray(authorization.handover.to_be_bytes().to_vec()),
                        ),
                        ("schema", Tag::Long(1)),
                        ("length", Tag::Int(payload.len() as i32)),
                        ("sha256", Tag::ByteArray(Sha256::digest(payload).to_vec())),
                    ]),
                )]),
            )]));
            receipt::append_receipt(world, guest, &received, Some(&authorization)).unwrap();
            guest.dimension = DIMENSION.into();
            guest.place([12.0, 70.0, 24.0]);
            guest.xp_level = 5;
            guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 1));
            playerdata::save_now(guest, world).unwrap();
            guest.xp_level = 41;
            guest.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
            playerdata::save_now(guest, world).unwrap();
        })
        .unwrap();
    assert!(
        playerdata::path(world, OWNER)
            .with_extension("dat_old")
            .is_file()
    );
    (authorization, request)
}

fn profile_file(world: &Path, authorization: &ReturnAuthorization) -> std::path::PathBuf {
    let key = Sha256::digest(PROFILE.as_bytes());
    world.join("players/data").join(format!(
        "{OWNER:032x}-{:032x}-{key:x}.hprofile",
        authorization.handover,
    ))
}

#[test]
#[ignore = "daily"]
fn reopened_current_owner_requires_trusted_resume_and_loads_latest_local_player() {
    let world = tempfile::tempdir().unwrap();
    let mut seeded = None;
    crate::game::native_guest_turn_tests::with_guests(|game| {
        seeded = Some(seed_committed_return(world.path(), game));
    });
    let (authorization, request) = seeded.unwrap();
    let owner = uuid_string(OWNER);
    let trusted = format!("{owner} {}", serde_json::to_string(&authorization).unwrap());
    let expect = format!(
        "{owner} {{\"authorization\":{}}}",
        serde_json::to_string(&authorization).unwrap()
    );
    let mut forged = authorization.clone();
    forged.revision = 10.try_into().unwrap();
    let wrong = format!("{owner} {}", serde_json::to_string(&forged).unwrap());
    let journal = Arc::new(Journal::open(world.path()).unwrap());
    let transfers = arrivals::Transfers::new(phase_settings());
    let expected = transfers.expected.clone();
    let resources = std::env::var_os("HORIZON_TEST_RESOURCES").unwrap().into();
    crate::game::native_guest_turn_tests::with_guests_configured(
        resources,
        |net| {
            net.listen = true;
            net.bind = Some(std::net::Ipv4Addr::LOCALHOST.into());
            net.arrivals = Some(expected);
            net.forwarding_secret = Some(KEY.to_vec());
            net.source_handovers = Some(journal.clone());
        },
        |game| {
            game.world_root = world.path().to_owned();
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.transfers = Some(transfers);
            hub.guests.remove(hub.guests.connection_index(12).unwrap());
            let port = hub.lan.port;
            assert_eq!(
                journal.fenced_handover(OWNER),
                Some(authorization.retired.handover)
            );
            assert!(forwarded_login(port, false).is_err());
            assert!(game.player_request("player-profile-resume", &trusted)["error"].is_string());
            assert_eq!(
                game.player_request("player-profile-expect-return", &expect)["ok"],
                true
            );
            let mut client = forwarded_login(port, true).unwrap();
            let conn = wait_for_join(game);
            assert!(!game.lan_guest_joined(conn, &[], [0.5, 64.0, 0.5]));
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert!(guest.held && !guest.placed);
            assert_eq!(guest.dimension, DIMENSION);
            assert!(guest.inventory.iter().all(Option::is_none));
            let blindness = read_held_view(&mut client);
            assert!(game.player_request("player-profile-resume", &wrong)["error"].is_string());
            let historical = game.player_request("player-import", &format!("{owner} {request}"));
            assert_eq!(historical["currently_placed"], false, "{historical}");
            let confirm = format!(
                "{owner} {} east west {} {} 9",
                arrivals::handover_text(authorization.handover),
                receipt::digest_hex(authorization.record_sha256),
                receipt::digest_hex(authorization.request_sha256)
            );
            let historical = game.player_request("player-confirm", &confirm);
            assert_eq!(historical["currently_placed"], false, "{historical}");

            let file = playerdata::path(world.path(), OWNER);
            let latest = std::fs::read(&file).unwrap();
            std::fs::write(&file, b"invalid current data").unwrap();
            assert!(game.player_request("player-profile-resume", &trusted)["error"].is_string());
            assert!(journal.fenced_handover(OWNER).is_some());
            std::fs::write(&file, &latest).unwrap();
            let profile = profile_file(world.path(), &authorization);
            let payload = std::fs::read(&profile).unwrap();
            std::fs::write(&profile, vec![0; payload.len()]).unwrap();
            assert!(
                game.player_request("player-profile-resume", &trusted)["error"]
                    .as_str()
                    .unwrap()
                    .contains("digest")
            );
            std::fs::write(&profile, payload).unwrap();

            let resumed = game.player_request("player-profile-resume", &trusted);
            assert_eq!(resumed["currently_placed"], true, "{resumed}");
            assert_eq!(resumed["retried"], false);
            read_resumed_view(&mut client, blindness);
            assert_eq!(
                game.player_request("player-profile-resume", &trusted)["retried"],
                true
            );
            assert!(game.player_request("player-profile-resume", &wrong)["error"].is_string());
            assert!(journal.fenced_handover(OWNER).is_none());
            let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
            assert!(!guest.held && guest.placed_in(DIMENSION));
            assert_eq!(guest.position, [12.0, 70.0, 24.0]);
            assert_eq!(guest.xp_level, 41);
            assert_eq!(guest.inventory[0].as_ref().unwrap().count, 3);
            assert_eq!(std::fs::read(&file).unwrap(), latest);
        },
    );
}
