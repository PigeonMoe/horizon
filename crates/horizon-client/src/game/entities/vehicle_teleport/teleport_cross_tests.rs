//! `/tp` of hanging entities (`BlockAttachedEntity.setPos`) and of any entity
//! kind into another dimension (`Entity.teleportCrossDimension`).
use crate::game::{Audio, Dim, Game, Options, Progress, Settings, execution_role, load_resources};
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use std::sync::Arc;

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
        Vec::new(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        crate::game::execution_role::ExecutionRole::IntegratedServer,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
            root.to_path_buf(),
        )),
        None,
    )
    .unwrap();
    game.reset_entity_residency();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    game
}

/// `BlockAttachedEntity.setPos`: the block it hangs in is the floor of the
/// requested position; its bounds follow from it.
#[test]
#[ignore = "daily"]
fn tp_moves_a_frame_and_a_painting() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:painting 0 2 0 {facing:0b}"));
    assert!(game.run_command("summon minecraft:item_frame 0 2 2 {Facing:3b}"));
    assert!(game.run_command("tp @e[type=minecraft:painting,limit=1] 3 2 0"));
    assert!(game.run_command("tp @e[type=minecraft:item_frame,limit=1] 3 2 2"));
    assert_eq!(game.paintings.len(), 1);
    assert_eq!(game.paintings[0].pos, [3, 2, 0]);
    assert_eq!(game.item_frames.len(), 1);
    assert_eq!(game.item_frames[0].pos, [3, 2, 2]);
}

/// `Entity.teleportTo(ServerLevel, ..)` -> `teleportCrossDimension` for a
/// target in another level than the command source's.
#[test]
#[ignore = "daily"]
fn every_entity_kind_follows_a_cross_dimension_tp() {
    let mut game = game();
    let back = game.enter_command_level(Dim::Nether);
    assert_eq!(game.dim, Dim::Nether);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(0, 0));
    if let Some(back) = back {
        game.enter_command_level(back);
    }
    for command in [
        "summon minecraft:armor_stand 0.5 2 0.5",
        "summon minecraft:item 1.5 2 0.5 {Item:{id:\"minecraft:stone\",count:1}}",
        "summon minecraft:item_frame 2.5 2 0.5 {Facing:3b}",
        "summon minecraft:arrow 3.5 2 0.5 {Motion:[0.0d,0.0d,0.0d]}",
        "summon minecraft:tnt 4.5 2 0.5",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    for (ty, x) in [
        ("minecraft:armor_stand", 10.5),
        ("minecraft:item", 11.5),
        ("minecraft:item_frame", 12.5),
        ("minecraft:arrow", 13.5),
        ("minecraft:tnt", 14.5),
    ] {
        let command =
            format!("execute in minecraft:the_nether run tp @e[type={ty},limit=1] {x} 64 0.5");
        assert!(game.run_command(&command), "{command}");
    }
    assert!(game.armor_stands.is_empty(), "the stand left the overworld");
    assert!(game.dropped.is_empty(), "the item left the overworld");
    assert!(game.item_frames.is_empty(), "the frame left the overworld");
    assert!(
        game.creatures.projectiles.is_empty(),
        "the arrow left the overworld"
    );
    assert!(game.tnts.is_empty(), "the tnt left the overworld");
    let back = game.enter_command_level(Dim::Nether);
    assert_eq!(game.dim, Dim::Nether);
    let stand = game.armor_stands.iter().next().expect("stand crossed");
    assert_eq!(
        [stand.position.x, stand.position.y, stand.position.z],
        [10.5, 64.0, 0.5]
    );
    let item = game.dropped.iter().next().expect("item crossed");
    assert_eq!(
        [item.position.x, item.position.y, item.position.z],
        [11.5, 64.0, 0.5]
    );
    let frame = game.item_frames.first().expect("frame crossed");
    assert_eq!(frame.pos, [12, 64, 0]);
    let arrow = game
        .creatures
        .projectiles
        .iter()
        .next()
        .expect("arrow crossed");
    assert_eq!(
        [arrow.position.x, arrow.position.y, arrow.position.z],
        [13.5, 64.0, 0.5]
    );
    let tnt = game.tnts.first().expect("tnt crossed");
    assert_eq!(
        [tnt.position.x, tnt.position.y, tnt.position.z],
        [14.5, 64.0, 0.5]
    );
    // `PrimedTnt.teleport`: the new entity has used a portal.
    assert!(tnt.used_portal, "the arrived tnt ignores its portal");
    if let Some(back) = back {
        game.enter_command_level(back);
    }
}

#[test]
#[ignore = "daily"]
fn a_conflicting_dragon_destination_preserves_the_source_vehicle_tree() {
    use super::{Target, TeleportMotion};
    use horizon_mod_runtime::attachments::{AttachmentType, Attachments, Holder};

    let mut game = game();
    game.enter_level(Dim::Nether).unwrap();
    assert!(game.run_command("summon minecraft:ender_dragon 15 65 15"));
    let destination_dragon = game.dragon.as_ref().unwrap().id;
    game.enter_level(Dim::Overworld).unwrap();
    assert!(game.run_command(concat!(
        "summon minecraft:pig 1 64 1 ",
        "{Passengers:[{id:\"minecraft:cow\"},{id:\"minecraft:ender_dragon\"}]}"
    )));
    let root = game
        .mobs
        .iter()
        .find(|mob| mob.extra.mount.is_none())
        .unwrap()
        .id;
    let [cow, dragon] = game.ordered_live_passengers(root).try_into().unwrap();
    assert_eq!(
        game.level_entity(dragon as i32),
        Some(Target::Dragon(dragon))
    );
    let vehicles = [Target::Mob(root), Target::Mob(cow), Target::Dragon(dragon)];
    let poses = vehicles.map(|target| {
        (
            game.target_position(target).unwrap(),
            game.target_rotation(target),
        )
    });
    let attachment = AttachmentType {
        namespace: "mymod",
        name: "vehicle_rider",
        default: || 0,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    let mut values = Attachments::default();
    attachment
        .set(&mut values, Holder::Entity(cow), 41)
        .unwrap();
    game.entity_attachments.insert(cow, values);

    let destination = [8.0, 72.0, 8.0];
    let error = game
        .teleport_vehicle_across(
            Target::Mob(root),
            Dim::Nether,
            destination,
            Some([90.0, 10.0]),
            true,
            TeleportMotion::Stop,
        )
        .unwrap_err();
    assert!(error.to_string().contains("already has an ender dragon"));
    assert_eq!(game.dim, Dim::Overworld);
    assert_eq!(game.ordered_live_passengers(root), [cow, dragon]);
    for (target, (position, rotation)) in vehicles.into_iter().zip(poses) {
        assert_eq!(game.target_position(target), Some(position));
        assert_eq!(game.target_rotation(target), rotation);
    }
    assert_eq!(
        attachment.get(
            game.entity_attachments.get_mut(&cow).unwrap(),
            Holder::Entity(cow)
        ),
        Ok(41),
    );
    game.enter_level(Dim::Nether).unwrap();
    assert_eq!(game.dragon.as_ref().unwrap().id, destination_dragon);
    assert!(game.mobs.is_empty());
    game.discard_resident_dragon();
    game.enter_level(Dim::Overworld).unwrap();

    assert!(
        game.teleport_vehicle_across(
            Target::Mob(root),
            Dim::Nether,
            destination,
            None,
            false,
            TeleportMotion::Stop,
        )
        .unwrap()
    );
    assert!(game.mobs.is_empty());
    assert!(game.dragon.as_ref().is_none());
    game.enter_level(Dim::Nether).unwrap();
    let root = game
        .mobs
        .iter()
        .find(|mob| mob.extra.mount.is_none())
        .unwrap()
        .id;
    let [cow, dragon] = game.ordered_live_passengers(root).try_into().unwrap();
    assert_eq!(game.target_position(Target::Mob(root)), Some(destination));
    assert_eq!(game.dragon.as_ref().unwrap().id, dragon);
    assert_eq!(
        attachment.get(
            game.entity_attachments.get_mut(&cow).unwrap(),
            Holder::Entity(cow)
        ),
        Ok(41),
    );
}

#[test]
#[ignore = "daily"]
fn a_conflicting_dragon_destination_still_accepts_the_root_after_rider_veto() {
    use super::{Target, TeleportMotion};
    use mod_api::{EntityTravelToDimensionEvent, Flow, Hooks, ServerHost};

    struct VetoCow;
    #[mod_api::__subscriptions]
    impl Hooks for VetoCow {
        fn entity_travel_to_dimension(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityTravelToDimensionEvent,
        ) -> Flow {
            if event.entity.kind(host).map(|kind| kind.id(host)).as_deref() == Some("minecraft:cow")
            {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("veto_cow", Box::new(VetoCow))].into(),
        || {
            let mut game = game();
            game.enter_level(Dim::Nether).unwrap();
            assert!(game.run_command("summon minecraft:ender_dragon 15 65 15"));
            let destination_dragon = game.dragon.as_ref().unwrap().id;
            game.enter_level(Dim::Overworld).unwrap();
            assert!(game.run_command(concat!(
                "summon minecraft:pig 1 64 1 ",
                "{Passengers:[{id:\"minecraft:cow\",Passengers:[{id:\"minecraft:ender_dragon\"}]}]}"
            )));
            let root = game
                .mobs
                .iter()
                .find(|mob| mob.extra.mount.is_none())
                .unwrap()
                .id;
            let [cow] = game.ordered_live_passengers(root).try_into().unwrap();
            let dragon = game.dragon.as_ref().unwrap().id;
            assert_eq!(game.ordered_live_passengers(cow), [dragon]);

            let destination = [8.0, 72.0, 8.0];
            assert!(
                game.teleport_vehicle_across(
                    Target::Mob(root),
                    Dim::Nether,
                    destination,
                    None,
                    false,
                    TeleportMotion::Stop,
                )
                .unwrap()
            );
            assert!(game.mob_object(root).is_none());
            assert_eq!(game.mob_object(cow).unwrap().extra.mount, None);
            assert_eq!(game.ordered_live_passengers(cow), [dragon]);
            game.enter_level(Dim::Nether).unwrap();
            assert_eq!(game.dragon.as_ref().unwrap().id, destination_dragon);
            assert_eq!(game.mobs.len(), 1);
            assert_eq!(
                game.target_position(Target::Mob(game.mobs[0].id)),
                Some(destination)
            );
        },
    );
}

#[test]
#[ignore = "daily"]
fn cross_dimension_vehicle_tp_vetoes_one_player_and_commits_the_other_once() {
    check_cross_dimension_vehicle(CrossDimensionVehicle::Command);
}

#[test]
#[ignore = "daily"]
fn cross_dimension_vehicle_rollback_moves_the_root_and_only_the_admitted_rider() {
    check_cross_dimension_vehicle(CrossDimensionVehicle::Rollback);
}

#[test]
#[ignore = "daily"]
fn cross_dimension_vehicle_rollback_refusal_recovers_only_the_denied_rider() {
    check_cross_dimension_vehicle(CrossDimensionVehicle::RefusedRollback);
}

#[test]
#[ignore = "daily"]
fn cross_dimension_vehicle_rollback_change_keeps_same_world_rewrites_local() {
    check_cross_dimension_vehicle(CrossDimensionVehicle::LocalRollback);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CrossDimensionVehicle {
    Command,
    Rollback,
    RefusedRollback,
    LocalRollback,
}

fn check_cross_dimension_vehicle(route: CrossDimensionVehicle) {
    use crate::game::{
        commands::Target, lan_actions::GuestCommand, native_entity_refs::tests::run_on_guest_stack,
        native_guest_turn_tests::with_guests,
    };
    use horizon_core::geometry::Vec3;
    use mod_api::{
        EntityMountEvent, EntityTeleportPreEvent, EntityTravelToDimensionEvent, Flow, Hooks, Host,
        Identifier, ModEntry, PlayerPositionCommittedEvent, PlayerTeleportPreEvent, ServerHost,
    };
    use std::sync::Mutex;

    const DENIED: u128 = 12;
    const ACCEPTED: u128 = 13;
    const ARRIVAL: [f64; 3] = [10.5, 72.0, 10.5];
    const RETURN: [f64; 3] = [3.5, 64.0, 3.5];
    const BOAT: u32 = 900;
    const CAMEL: u32 = 901;

    #[derive(Clone, Debug, PartialEq)]
    struct Move {
        player: u128,
        from: [f64; 3],
        to: [f64; 3],
    }
    #[derive(Default)]
    struct Seen {
        dismounted: Vec<u32>,
        proposed: Vec<Move>,
        committed: Vec<Move>,
        rollback_redirects: usize,
    }
    struct Protection(Arc<Mutex<Seen>>, CrossDimensionVehicle);
    #[mod_api::__subscriptions]
    impl Hooks for Protection {
        fn entity_travel_to_dimension(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityTravelToDimensionEvent,
        ) -> Flow {
            if self.1 == CrossDimensionVehicle::RefusedRollback && event.entity.id() == BOAT {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn entity_teleport_pre(
            &self,
            _: &dyn Host,
            event: &mut EntityTeleportPreEvent<'_>,
        ) -> Flow {
            assert_ne!(self.1, CrossDimensionVehicle::Command);
            assert_eq!(event.entity, BOAT);
            assert_eq!(event.dimension, Dim::Overworld.id());
            assert_eq!(event.cause, "vehicle_rollback");
            assert_eq!(*event.to, RETURN);
            if self.1 != CrossDimensionVehicle::LocalRollback {
                *event.to_dimension = Identifier::parse(Dim::Nether.id()).unwrap();
            }
            *event.to = ARRIVAL;
            *event.to_rotation = [30.0, 0.0];
            self.0.lock().unwrap().rollback_redirects += 1;
            Flow::Continue
        }

        fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
            if !event.mounting && event.kind == "minecraft:player" {
                self.0.lock().unwrap().dismounted.push(event.entity);
            }
            Flow::Continue
        }

        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            if event.from_level == event.to_level {
                return Flow::Continue;
            }
            assert_eq!(event.from_level, Dim::Overworld.id());
            assert_eq!(event.to_level, Dim::Nether.id());
            let rider = host.player(event.player).unwrap().entity();
            assert_eq!(rider.position(host), Some(event.from));
            assert_eq!(rider.vehicle(host), None);
            self.0.lock().unwrap().proposed.push(Move {
                player: event.player,
                from: event.from,
                to: event.to,
            });
            if event.player == DENIED {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn player_position_committed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerPositionCommittedEvent<'_>,
        ) {
            if event.from_level == event.to_level {
                return;
            }
            assert_eq!(event.from_level, Dim::Overworld.id());
            assert_eq!(event.to_level, Dim::Nether.id());
            let player = host.players().acting_player().unwrap();
            assert_eq!(player.uuid(), event.player);
            assert_eq!(player.entity().position(host), Some(event.to));
            self.0.lock().unwrap().committed.push(Move {
                player: event.player,
                from: event.from,
                to: event.to,
            });
        }
    }

    run_on_guest_stack(move || {
        for nested in [false, true] {
            let seen = Arc::new(Mutex::new(Seen::default()));
            mod_platform::testing::with_mods(
                vec![ModEntry::new(
                    "passenger_teleport_protection",
                    Box::new(Protection(seen.clone(), route)),
                )]
                .into(),
                || {
                    with_guests(|game| {
                        game.options.dedicated = Some(Vec::new());
                        game.set_allow_commands(true);
                        game.world
                            .chunks
                            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                        game.block_ticks
                            .add_empty_chunk(ChunkPos::new(0, 0))
                            .unwrap();
                        assert!(game.run_command("forceload add 0 0"));
                        let stone = game
                            .pack
                            .assets
                            .registry
                            .default_state("minecraft:stone")
                            .unwrap();
                        game.world.set(6, 63, 7, stone).unwrap();
                        game.world.set(5, 63, 6, stone).unwrap();
                        let origin = Vec3::new(6.5, 64.0, 6.5);
                        let boat = crate::game::boats::Boat::new(BOAT, "oak".into(), origin, 0.0);
                        game.boats.push(boat);
                        let hull_uuid = game.uuid_for(BOAT);
                        let hull = game.entity(BOAT).unwrap();
                        let mount = if nested {
                            let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                            let mut camel =
                                horizon_client::mobs::Mob::new(CAMEL, kind, origin, 0.0);
                            camel.extra.set_saddled(true);
                            game.mobs.push(camel);
                            let camel = game.entity(CAMEL).unwrap();
                            assert!(game.entity_start_riding(camel, hull, true));
                            Some(camel)
                        } else {
                            None
                        };
                        for (uuid, yaw) in [(DENIED, 0.0), (ACCEPTED, 90.0)] {
                            game.lan
                                .as_mut()
                                .unwrap()
                                .hub
                                .update_guest(uuid as u32, |guest| guest.yaw = yaw);
                            let player = game.player(uuid).unwrap().entity();
                            let seat = if uuid == ACCEPTED {
                                mount.unwrap_or(hull)
                            } else {
                                hull
                            };
                            assert!(game.entity_start_riding(player, seat, true));
                            game.guest_boarded(uuid as u32, None);
                        }
                        game.position_riders();
                        let expected_move = |uuid, from: Vec3| {
                            let to = super::passenger_destination(
                                origin,
                                Vec3::new(ARRIVAL[0], ARRIVAL[1], ARRIVAL[2]),
                                from,
                            );
                            Move {
                                player: uuid,
                                from: from.components(),
                                to: to.components(),
                            }
                        };
                        let mut expected = [DENIED, ACCEPTED].map(|uuid| {
                            let yaw = game.command_guest(uuid as u32).unwrap().yaw;
                            let from = game.boat_player_dismount_target(0, yaw);
                            expected_move(uuid, from)
                        });
                        assert_ne!(expected[0].to, expected[1].to);
                        if route == CrossDimensionVehicle::Command {
                            assert!(game.run_guest_command(GuestCommand {
                                conn: ACCEPTED as u32,
                                command: "execute in minecraft:the_nether run tp @e[type=minecraft:oak_boat,limit=1] 10.5 72 10.5".into(),
                                permission: 4,
                                integrated: true,
                                quiet: false,
                            }));
                        } else {
                            let [x, y, z] = RETURN;
                            let recovery = game.edit_vehicle_rollback(
                                BOAT,
                                DENIED,
                                (origin, [0.0; 2]),
                                (Vec3::new(x, y, z), [0.0; 2]),
                            );
                            assert!(game.finish_vehicle_rollback(Target::Boat(BOAT), recovery));
                        }
                        game.run_level_teleports();
                        if route == CrossDimensionVehicle::LocalRollback {
                            let denied = game.command_guest(DENIED as u32).unwrap();
                            assert_eq!(denied.dimension, Dim::Overworld.id());
                            assert_eq!(denied.vehicle, Some(BOAT));
                            let accepted = game.command_guest(ACCEPTED as u32).unwrap();
                            assert_eq!(accepted.dimension, Dim::Overworld.id());
                            assert_eq!(
                                accepted.vehicle,
                                Some(mount.map_or(BOAT, |mount| mount.id()))
                            );
                            assert_eq!(game.boats[0].position.components(), ARRIVAL);
                            assert_eq!(game.boats[0].yaw, 30.0);
                            let seen = seen.lock().unwrap();
                            assert_eq!(seen.rollback_redirects, 1);
                            assert!(seen.proposed.is_empty() && seen.committed.is_empty());
                            return;
                        }
                        if route == CrossDimensionVehicle::RefusedRollback {
                            let denied = game.command_guest(DENIED as u32).unwrap();
                            assert_eq!(denied.dimension, Dim::Overworld.id());
                            assert_eq!((denied.position, denied.vehicle), (RETURN, None));
                            let accepted = game.command_guest(ACCEPTED as u32).unwrap();
                            assert_eq!(accepted.dimension, Dim::Overworld.id());
                            assert_ne!(accepted.position, RETURN);
                            if !nested {
                                assert_eq!(accepted.vehicle, None);
                            }
                            assert_eq!(game.boats[0].position, origin);
                            game.enter_level(Dim::Nether).unwrap();
                            assert!(game.boats.is_empty());
                            let seen = seen.lock().unwrap();
                            assert_eq!(seen.rollback_redirects, 1);
                            assert!(seen.proposed.is_empty() && seen.committed.is_empty());
                            return;
                        }
                        if nested {
                            let [x, y, z] = seen.lock().unwrap().proposed[1].from;
                            expected[1] = expected_move(ACCEPTED, Vec3::new(x, y, z));
                        }

                        let denied = game.command_guest(DENIED as u32).unwrap();
                        assert_eq!(denied.dimension, Dim::Overworld.id());
                        assert_eq!((denied.position, denied.vehicle), (expected[0].from, None));
                        let accepted = game.command_guest(ACCEPTED as u32).unwrap();
                        assert_eq!(accepted.dimension, Dim::Nether.id());
                        assert_eq!(accepted.position, expected[1].to);
                        let new_mount = accepted.vehicle.unwrap();
                        assert!(game.boats.is_empty());
                        game.enter_level(Dim::Nether).unwrap();
                        assert_eq!(game.boats.len(), 1);
                        let new_hull = game.boats[0].id;
                        assert_eq!(game.uuid_for(new_hull), hull_uuid);
                        assert_eq!(game.boats[0].position.components(), ARRIVAL);
                        if route == CrossDimensionVehicle::Rollback {
                            assert_eq!(game.boats[0].yaw, 30.0);
                        }
                        assert_eq!(game.seated_passengers(new_hull).len(), 1);
                        if nested {
                            assert_eq!(
                                game.mob_object(new_mount).unwrap().extra.mount,
                                Some(new_hull)
                            );
                            assert_eq!(
                                game.seated_passengers(new_mount),
                                [(0, ACCEPTED as u32, true)]
                            );
                        } else {
                            assert_eq!(new_mount, new_hull);
                        }
                        assert!(
                            game.run_command("execute if entity @a[name=guest13,distance=..128]")
                        );
                        assert!(
                            !game.run_command("execute if entity @a[name=guest12,distance=..128]")
                        );
                        let seen = seen.lock().unwrap();
                        assert_eq!(
                            seen.rollback_redirects,
                            usize::from(route != CrossDimensionVehicle::Command)
                        );
                        let dismounted = if nested {
                            [DENIED, ACCEPTED]
                        } else {
                            [ACCEPTED, DENIED]
                        };
                        assert_eq!(seen.dismounted, dismounted.map(|uuid| uuid as u32));
                        assert_eq!(seen.proposed, expected);
                        assert_eq!(seen.committed, [expected[1].clone()]);
                    });
                },
            );
        }
    });
}

#[test]
#[ignore = "daily"]
fn cross_dimension_vehicle_rollback_reserves_the_dragon_during_source_leave() {
    use crate::game::{commands::Target, native_guest_turn_tests::with_guests};
    use horizon_core::geometry::Vec3;
    use horizon_guest_stack::run_on_guest_stack;
    use mod_api::{
        EntityJoinLevelEvent, EntityLeaveLevelEvent, EntityTeleportPreEvent, Flow, Hooks, Host,
        Identifier, ModEntry, ServerHost,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct Seen {
        transitions: Vec<(bool, String)>,
        summons: Vec<bool>,
    }

    struct Rival(Arc<Mutex<Seen>>);
    #[mod_api::__subscriptions]
    impl Hooks for Rival {
        fn entity_teleport_pre(
            &self,
            _: &dyn Host,
            event: &mut EntityTeleportPreEvent<'_>,
        ) -> Flow {
            if event.cause == "vehicle_rollback" {
                *event.to_dimension = Identifier::parse(Dim::Nether.id()).unwrap();
            }
            Flow::Continue
        }

        fn entity_leave_level(
            &self,
            host: &mut dyn ServerHost,
            event: &mut EntityLeaveLevelEvent<'_>,
        ) {
            if event.dimension != Dim::Overworld.id() {
                return;
            }
            self.0
                .lock()
                .unwrap()
                .transitions
                .push((false, event.kind.into()));
            if event.kind == "minecraft:oak_boat" {
                let ordinary = host.commands().run_command(
                    "execute in minecraft:the_nether run summon minecraft:ender_dragon 15 65 15",
                );
                let with_nbt = host.commands().run_command(
                    "execute in minecraft:the_nether run summon minecraft:ender_dragon 15 65 15 {}",
                );
                let control = host.commands().run_command(
                    "execute in minecraft:the_nether run summon minecraft:pig 15 65 15 {}",
                );
                self.0
                    .lock()
                    .unwrap()
                    .summons
                    .extend([ordinary, with_nbt, control]);
            }
        }

        fn entity_join_level(
            &self,
            _: &mut dyn ServerHost,
            event: &mut EntityJoinLevelEvent<'_>,
        ) -> Flow {
            if event.dimension == Dim::Nether.id() {
                self.0
                    .lock()
                    .unwrap()
                    .transitions
                    .push((true, event.kind.into()));
            }
            Flow::Continue
        }
    }

    run_on_guest_stack(|| {
        let seen = Arc::new(Mutex::new(Seen::default()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new("dragon_rival", Box::new(Rival(seen.clone())))].into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    // The fixture inserts guests 12 and 13 without allocating their ids.
                    game.next_entity = 1000;
                    game.build_registries();
                    game.set_allow_commands(true);
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    if game.ui.difficulty == 0 {
                        assert!(game.run_command("difficulty normal"));
                    }
                    assert!(game.run_command(concat!(
                    "summon minecraft:oak_boat 1 64 1 ",
                    "{Passengers:[{id:\"minecraft:cow\",Passengers:[{id:\"minecraft:ender_dragon\"}]}]}"
                )));
                    let boat = game.boats[0].id;
                    let boat_uuid = game.uuid_for(boat);
                    let dragon = game.dragon.as_ref().unwrap().id;
                    let dragon_uuid = game.uuid_for(dragon);
                    let [cow] = game.ordered_live_passengers(boat).try_into().unwrap();
                    assert_eq!(game.ordered_live_passengers(cow), [dragon]);

                    let recovery = game.edit_vehicle_rollback(
                        boat,
                        0,
                        (game.boats[0].position, [0.0; 2]),
                        (Vec3::new(8.5, 72.0, 8.5), [0.0; 2]),
                    );
                    assert!(game.finish_vehicle_rollback(Target::Boat(boat), recovery));
                    assert_eq!(game.dim, Dim::Overworld);
                    assert!(game.boats.is_empty() && game.dragon.is_none());
                    game.enter_level(Dim::Nether).unwrap();
                    assert_eq!(game.boats.len(), 1);
                    let boat = game.boats[0].id;
                    assert_eq!(game.uuid_for(boat), boat_uuid);
                    let [cow] = game.ordered_live_passengers(boat).try_into().unwrap();
                    let dragon = game.dragon.as_ref().unwrap().id;
                    assert_eq!(game.uuid_for(dragon), dragon_uuid);
                    assert_eq!(game.ordered_live_passengers(cow), [dragon]);
                    assert!(!game.dragon.has_reserved_arrival());
                    {
                        let seen = seen.lock().unwrap();
                        assert_eq!(seen.summons, [false, false, true]);
                        assert_eq!(
                            seen.transitions
                                .iter()
                                .map(|(join, kind)| (*join, kind.as_str()))
                                .collect::<Vec<_>>(),
                            [
                                (false, "minecraft:oak_boat"),
                                (true, "minecraft:pig"),
                                (false, "minecraft:cow"),
                                (false, "minecraft:ender_dragon"),
                                (true, "minecraft:ender_dragon"),
                                (true, "minecraft:cow"),
                                (true, "minecraft:oak_boat"),
                            ],
                        );
                    }
                    game.discard_resident_dragon();
                    assert!(game.native_run_command(
                    "execute in minecraft:the_nether run summon minecraft:ender_dragon 15 65 15 {}"
                ));
                })
            },
        );
    });
}

/// `Level.isInSpawnableBounds` on `BlockPos.containing`: the lower bounds are
/// inside, the upper bounds outside, so `/tp` accepts the world border
/// coordinates. Pins the bound (already true before the cross-dimension move).
#[test]
#[ignore = "daily"]
fn tp_accepts_the_spawnable_border_coordinates() {
    let mut game = game();
    assert!(game.run_command("summon minecraft:armor_stand 0.5 2 0.5"));
    for command in [
        "tp @e[type=minecraft:armor_stand,limit=1] -30000000 64 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 0 -20000000 0",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    for command in [
        "tp @e[type=minecraft:armor_stand,limit=1] -30000001 64 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 0 -20000001 0",
        "tp @e[type=minecraft:armor_stand,limit=1] 30000000 64 0",
    ] {
        assert!(!game.run_command(command), "{command}");
    }
}
