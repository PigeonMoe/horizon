//! A projectile that strikes an item frame hurts it (#11383): `ItemFrame.hurtServer`
//! gives the item up, an empty frame breaks, and a fixed one refuses the hit.
use crate::game::{Audio, Game, Options, Progress, Settings, execution_role, load_resources};
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
            root.clone(),
        )),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    for command in [
        "forceload add 0 0",
        "fill 0 63 0 15 63 15 stone",
        "fill 4 64 0 4 66 15 stone",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game
}

/// An arrow from `(0.5, 64.5, z)` flying east into the wall at x = 4.
fn shoot(game: &mut Game<'_>, z: f64) {
    assert!(game.run_command(&format!(
        "summon minecraft:arrow 0.5 64.5 {z} {{Motion:[1.0d,0.0d,0.0d]}}"
    )));
    for _ in 0..10 {
        game.tick_projectiles().unwrap();
    }
}

fn dropped(game: &Game<'_>, item: &str) -> usize {
    game.dropped
        .iter()
        .filter(|d| &*d.stack.item == item)
        .count()
}

#[test]
#[ignore = "daily"]
fn an_arrow_knocks_the_item_out_then_breaks_the_frame() {
    let mut game = game();
    // On the wall's west face (Facing 4: west).
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 64.5 6.5 {Facing:4b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    assert_eq!(game.item_frames.len(), 1);
    shoot(&mut game, 6.5);
    assert_eq!(game.item_frames.len(), 1, "the first hit leaves the frame");
    assert!(game.item_frames[0].item.is_none(), "the item came out");
    assert_eq!(dropped(&game, "minecraft:diamond"), 1);
    // The empty frame breaks and drops itself.
    shoot(&mut game, 6.5);
    assert!(game.item_frames.is_empty());
    assert_eq!(dropped(&game, "minecraft:item_frame"), 1);
}

#[test]
#[ignore = "daily"]
fn a_fixed_frame_refuses_the_arrow() {
    let mut game = game();
    assert!(game.run_command(
        "summon minecraft:item_frame 3.5 64.5 9.5 {Facing:4b,Fixed:1b,Item:{id:\"minecraft:diamond\",count:1}}"
    ));
    shoot(&mut game, 9.5);
    assert_eq!(game.item_frames.len(), 1);
    assert!(
        game.item_frames[0].item.is_some(),
        "a fixed frame keeps its item"
    );
    assert_eq!(dropped(&game, "minecraft:diamond"), 0);
    // The frame took the arrow's hit and refused it, so the arrow turned back
    // (`AbstractArrow.onHitEntity`'s deflection) instead of reaching the wall.
    assert!(
        game.creatures
            .projectiles
            .iter()
            .all(|p| p.position.x < 3.9),
        "the arrow went through the frame"
    );
}

#[test]
#[ignore = "daily"]
fn a_normal_arrow_reports_only_real_damage_hanging_break_and_swept_portal_contact() {
    use mod_api::{
        ArrowContactPostEvent, Flow, HangingBreakEvent, Hooks, LivingIncomingDamageEvent,
        ServerHost,
    };
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct Proof {
        deny: AtomicBool,
        contacts: Mutex<Vec<(u32, u32, String, [f64; 3])>>,
    }

    struct Probe(Arc<Proof>);

    #[mod_api::__subscriptions]
    impl Hooks for Probe {
        fn arrow_contact_post(
            &self,
            host: &mut dyn ServerHost,
            event: &mut ArrowContactPostEvent<'_>,
        ) {
            assert!(matches!(
                event.dimension,
                "minecraft:overworld" | "minecraft:the_end"
            ));
            self.0.contacts.lock().unwrap().push((
                event.projectile,
                event.owner,
                event.contact.into(),
                event.at,
            ));
            if event.contact == "portal" {
                let arrow = host
                    .entity(event.projectile)
                    .expect("portal arrow is still present");
                assert!(arrow.discard(host));
            }
        }

        fn hanging_break(&self, _: &mut dyn ServerHost, _: &mut HangingBreakEvent<'_>) -> Flow {
            if self.0.deny.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn living_incoming_damage(
            &self,
            _: &mut dyn mod_api::LevelRandomHost,
            _: &mut LivingIncomingDamageEvent<'_>,
        ) -> Flow {
            if self.0.deny.load(Ordering::Relaxed) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    let proof = Arc::new(Proof::default());
    let mods = vec![mod_api::ModEntry::new(
        "arrow_contact_probe",
        Box::new(Probe(proof.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut game = game();
        game.options.dedicated = Some(Vec::new());
        game.world_root = game.save.root.clone();
        game.reset_entity_residency();
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        game.sync_tick_chunks();
        game.update_chunk_tickets();
        let summon = |game: &mut Game<'_>, command: &str| {
            assert!(
                game.as_console(|game| game.run_command(command)),
                "{command}"
            );
        };
        let spawn = |game: &mut Game<'_>, kind: &str, position, nbt| {
            let dimension = game.dim.id();
            let host: &mut dyn ServerHost = game;
            host.summon(dimension, kind, position, nbt)
                .unwrap_or_else(|| panic!("could not summon {kind} in {dimension}"))
        };
        let nbt = mod_api::Compound::from([("NoAI".into(), mod_api::Tag::Byte(1))]);
        spawn(&mut game, "minecraft:pig", [2.5, 64.0, 2.5], nbt.clone());
        let shooter = spawn(&mut game, "minecraft:cow", [13.5, 64.0, 13.5], nbt);
        let throw = |game: &mut Game<'_>, z, flight_ticks| {
            let id = spawn(
                game,
                "minecraft:arrow",
                [0.5, 64.5, z],
                mod_api::Compound::new(),
            );
            let entity = game
                .entity(id)
                .unwrap_or_else(|| panic!("arrow {id} on lane {z} should be resident"));
            let arrow = game
                .projectile(entity)
                .expect("summoned entity is a projectile");
            let owner = game.entity(shooter).expect("shooter in level");
            assert!(arrow.set_owner(game, Some(owner)));
            assert!(entity.set_velocity(game, [1.0, 0.0, 0.0]));
            for _ in 0..flight_ticks {
                game.tick_projectiles().unwrap();
            }
            id
        };

        proof.deny.store(true, Ordering::Relaxed);
        throw(&mut game, 2.5, 3);
        assert!(proof.contacts.lock().unwrap().is_empty());
        let pig = game
            .mobs
            .iter()
            .find(|m| m.kind.id == "minecraft:pig")
            .unwrap();
        assert_eq!(pig.health, 10.0);
        proof.deny.store(false, Ordering::Relaxed);
        let hit = throw(&mut game, 2.5, 3);
        let heard = std::mem::take(&mut *proof.contacts.lock().unwrap());
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.as_str()),
            (hit, shooter, "damage")
        );

        summon(
            &mut game,
            "summon minecraft:item_frame 3.5 64.5 6.5 {Facing:4b}",
        );
        proof.deny.store(true, Ordering::Relaxed);
        throw(&mut game, 6.5, 4);
        assert_eq!(game.item_frames.len(), 1);
        assert!(proof.contacts.lock().unwrap().is_empty());
        proof.deny.store(false, Ordering::Relaxed);
        let frame = throw(&mut game, 6.5, 4);
        assert!(game.item_frames.is_empty());
        let heard = std::mem::take(&mut *proof.contacts.lock().unwrap());
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.as_str()),
            (frame, shooter, "hanging_break")
        );

        summon(
            &mut game,
            "summon minecraft:painting 3.5 64.5 9.5 {block_pos:[I;3,64,9],facing:1b,variant:\"minecraft:kebab\"}",
        );
        assert_eq!(game.paintings.len(), 1);
        let painting = throw(&mut game, 9.5, 4);
        assert!(
            game.paintings.is_empty(),
            "the arrow should break this painting"
        );
        let heard = std::mem::take(&mut *proof.contacts.lock().unwrap());
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.as_str()),
            (painting, shooter, "hanging_break")
        );

        summon(
            &mut game,
            "setblock 2 64 12 minecraft:nether_portal[axis=z]",
        );
        let portal = throw(&mut game, 12.5, 3);
        assert!(proof.contacts.lock().unwrap().is_empty());
        game.find_entity_portals();
        let heard = std::mem::take(&mut *proof.contacts.lock().unwrap());
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.as_str()),
            (portal, shooter, "portal")
        );
        assert!(game.entity(portal).is_none());
        assert!(!game.portals.entities.contains_key(&portal));
        game.find_entity_portals();
        assert!(proof.contacts.lock().unwrap().is_empty());

        game.dim = crate::game::Dim::End;
        game.reset_entity_residency();
        summon(&mut game, "setblock 2 64 15 minecraft:end_gateway");
        let gateway = throw(&mut game, 15.5, 3);
        game.find_end_gateway_portals();
        let heard = std::mem::take(&mut *proof.contacts.lock().unwrap());
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(
            (heard[0].0, heard[0].1, heard[0].2.as_str()),
            (gateway, shooter, "portal")
        );
        assert!(game.entity(gateway).is_none());
        assert!(!game.gateway_available([2, 64, 15]));
        let second = throw(&mut game, 15.5, 3);
        game.find_end_gateway_portals();
        assert!(proof.contacts.lock().unwrap().is_empty());
        assert!(game.entity(second).is_some());
    });
}
