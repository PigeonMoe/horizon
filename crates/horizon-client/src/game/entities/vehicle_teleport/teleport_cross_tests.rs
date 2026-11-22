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
