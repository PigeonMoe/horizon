//! `player.place_block` against real LAN guests, through the public player handle, with a
//! literal test for each refusal.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use mod_api::{EntityRef, PlayerWorldRefusal as Refusal, WorldEpoch};

/// The API's value for a block state of the world.
fn api_state(game: &super::Game<'_>, name: &str) -> mod_api::BlockState {
    use super::api_values::TryIntoApi;
    game.pack
        .assets
        .registry
        .default_state(name)
        .unwrap()
        .try_into_api()
        .unwrap()
}

fn loaded(game: &mut super::Game<'_>) {
    // The write is `level_set_block`'s, which an authoritative server only takes.
    game.options.dedicated = Some(Vec::new());
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
}

#[test]
#[ignore = "daily"]
fn device_placement_can_authorize_an_offline_owner_without_writing() {
    use mod_api::{DevicePlacementPreEvent, Flow, Hooks, Host, ModEntry};
    use std::sync::{Arc, Mutex};

    const TARGET: [i32; 3] = [4, 70, 2];
    const OFFLINE: u128 = 91;
    const OPERATOR: u128 = 92;
    struct Claims(Arc<Mutex<Vec<(u128, bool)>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Claims {
        fn device_placement_pre(
            &self,
            host: &dyn Host,
            event: &mut DevicePlacementPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.pos, TARGET);
            assert!(host.player(event.owner).is_none());
            self.0.lock().unwrap().push((event.owner, event.operator));
            if event.operator {
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "device-policy-fixture",
            Box::new(Claims(calls.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                loaded(game);
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                let level = game.level(game.dim.id()).unwrap();
                game.lan.as_mut().unwrap().hub.ops.insert(OPERATOR, 4);
                game.lan.as_mut().unwrap().hub.spawn_protection = 0;
                assert!(!level.allow_device_placement(game, OFFLINE, TARGET));
                assert!(level.allow_device_placement(game, OPERATOR, TARGET));
                assert!(!level.allow_device_placement(game, OPERATOR, [1_000, 70, 2]));

                game.level.world_spawn = Some(TARGET);
                game.level.world_spawn_dimension = Some(game.dim.id().to_owned());
                game.lan.as_mut().unwrap().hub.spawn_protection = 2;
                assert!(!level.allow_device_placement(game, OFFLINE, TARGET));
                assert!(level.allow_device_placement(game, OPERATOR, TARGET));
                assert_eq!(
                    level.block_state(game, TARGET),
                    Some(api_state(game, "minecraft:air"))
                );
            })
        },
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [(OFFLINE, false), (OPERATOR, true), (OPERATOR, true)]
    );
}

mod device_protection {
    use mod_api::{EntityChangeBlockPreEvent, Flow, Hooks, ServerHost};

    #[derive(Default)]
    struct Claims;

    #[mod_api::horizon_mod]
    impl Hooks for Claims {
        fn entity_change_block_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityChangeBlockPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.player, Some(13));
            assert_eq!(event.kind, "minecraft:player");
            match event.pos {
                [2, 70, 2] => Flow::Cancel,
                [3, 70, 2] => Flow::Continue,
                _ => panic!("unexpected block protection check at {:?}", event.pos),
            }
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_device_write_checks_the_real_player_and_the_expected_block_before_changing_it() {
    use mod_api::{BlockUpdateFlags, ModEntry};

    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "device-protection",
            device_protection::hooks(),
        )]
        .into(),
        || {
            with_guests(|game| {
                loaded(game);
                let player = game.player(13).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                let air = api_state(game, "minecraft:air");
                let water = api_state(game, "minecraft:water");
                let protected = [2, 70, 2];
                let open = [3, 70, 2];
                let spawn = [4, 70, 2];
                let flags = BlockUpdateFlags::CLIENTS | BlockUpdateFlags::KNOWN_SHAPE;

                assert_eq!(
                    level.replace_block_for_player(game, player, protected, air, water, flags),
                    Ok(false)
                );
                assert_eq!(level.block_state(game, protected), Some(air));

                assert_eq!(
                    level.replace_block_for_player(game, player, open, water, air, flags),
                    Ok(false)
                );
                assert_eq!(
                    level.replace_block_for_player(game, player, open, air, air, flags),
                    Ok(true)
                );
                assert_eq!(level.block_state(game, open), Some(air));
                assert_eq!(
                    level.replace_block_for_player(game, player, open, air, water, flags),
                    Ok(true)
                );
                assert_eq!(level.block_state(game, open), Some(water));

                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                game.level.world_spawn = Some(spawn);
                game.level.world_spawn_dimension = Some(game.dim.id().to_owned());
                let host = game.lan.as_mut().unwrap();
                host.hub.ops.insert(12, 4);
                host.hub.spawn_protection = 2;
                assert_eq!(
                    level.replace_block_for_player(game, player, spawn, air, water, flags),
                    Ok(false)
                );
                assert_eq!(level.block_state(game, spawn), Some(air));
            })
        },
    );
}

/// A bed written for a player gets its head (`setPlacedBy`).
#[test]
#[ignore = "daily"]
fn a_block_placed_as_a_player_runs_set_placed_by() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let bed = game
            .pack
            .assets
            .registry
            .default_state("minecraft:red_bed")
            .unwrap();
        let api_bed = api_state(game, "minecraft:red_bed");
        assert_eq!(
            b.place_block(game, level, [2, 70, 2], api_bed, true),
            Ok(())
        );
        assert_eq!(game.world.block(2, 70, 2), bed);
        let head = game.world.block(2, 70, 1);
        let facts = game.res.facts();
        assert_eq!(
            facts.get(head).name,
            facts.get(bed).name,
            "the head was placed"
        );
    });
}

/// Each refusal is its own name and writes nothing.
#[test]
#[ignore = "daily"]
fn each_place_block_refusal_has_its_own_name_and_writes_nothing() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let level = game.level(game.dim.id()).unwrap();
        let stone = api_state(game, "minecraft:stone");
        let empty = game.world.block(3, 70, 3);
        let at = [2, 70, 2];
        assert_eq!(b.place_block(game, level, at, stone, false), Ok(()));
        assert_eq!(
            b.place_block(game, level, at, stone, false),
            Err(Refusal::Unchanged),
            "stone already stands there"
        );
        for pos in [[2, 400, 2], [2, -100, 2], [30_000_000, 70, 2]] {
            assert_eq!(
                b.place_block(game, level, pos, stone, true),
                Err(Refusal::OutOfWorld { pos })
            );
        }
        let far = [1600, 70, 1600];
        assert_eq!(
            b.place_block(game, level, far, stone, true),
            Err(Refusal::NotLoaded { pos: far })
        );
        // A mod cannot make a state the world lacks; the game's own check names one.
        let nothing = horizon_world::registry::BlockStateId(u32::MAX);
        assert_eq!(
            game.player_place_block(b, level, [3, 70, 3], nothing, true),
            Err(Refusal::UnknownState)
        );
        let old =
            mod_api::LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
        assert_eq!(
            b.place_block(game, old, [3, 70, 3], stone, true),
            Err(Refusal::StaleLevel)
        );
        let gone = mod_api::__private::game::player_ref(
            EntityRef::new(
                WorldEpoch::new(b.entity().epoch().get() + 1),
                b.entity().id(),
            ),
            b.uuid(),
        );
        assert_eq!(
            gone.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Stale)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .update_guest(13, |g| g.dead = true)
            .unwrap();
        assert_eq!(
            b.place_block(game, level, [3, 70, 3], stone, true),
            Err(Refusal::Dead)
        );
        assert_eq!(game.world.block(3, 70, 3), empty, "nothing written");
    });
}

/// A level that is open but is not the one the player is in is `OtherLevel`.
#[test]
#[ignore = "daily"]
fn another_level_is_not_where_the_player_is() {
    with_guests(|game| {
        loaded(game);
        let b = game.player(13).unwrap();
        let overworld = game.level("minecraft:overworld").unwrap();
        let stone = api_state(game, "minecraft:stone");
        game.enter_level(super::Dim::Nether).unwrap();
        assert_eq!(
            b.place_block(game, overworld, [2, 70, 2], stone, true),
            Err(Refusal::OtherLevel)
        );
    });
}

/// On a client it is `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_place_a_block_as_a_player() {
    with_client_game(|game| {
        let level = mod_api::LevelRef::new(WorldEpoch::new(0), game.dim.key());
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        let stone = api_state(game, "minecraft:stone");
        assert_eq!(
            ghost.place_block(game, level, [2, 70, 2], stone, true),
            Err(Refusal::Client)
        );
    });
}
