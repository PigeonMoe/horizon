use horizon_core::nbt::Tag;
use mod_api::{Compound, EntityUuid, Host, LeashHolder, ServerHost};
use std::{collections::HashSet, sync::Arc};

#[test]
#[ignore = "daily"]
fn a_live_arrow_holds_and_pulls_a_leash_and_restores_by_uuid() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.build_registries();
        game.set_allow_commands(true);
        game.world_root = game.save.root.clone();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let mut chunk = horizon_client::chunk::Chunk::empty(0, 0);
        for x in 0..16 {
            for z in 0..16 {
                chunk.set(x, 63, z, stone.0);
            }
        }
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.block_ticks
            .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0))
            .unwrap();
        assert!(game.as_console(|game| game.run_command("forceload add 0 0")));
        game.sync_tick_chunks();
        game.update_chunk_tickets();
        assert!(game.tickets.levels().entity_ticking_at(4, 5));

        let spawn = |host: &mut dyn ServerHost, kind: &str, x, nbt| {
            host.summon("minecraft:overworld", kind, [x, 64.0, 5.5], nbt)
                .unwrap_or_else(|| panic!("server could not summon {kind}"))
        };
        let arrow_id = spawn(game, "minecraft:arrow", 11.5, Compound::new());
        let mut bat_flags = Compound::new();
        bat_flags.insert("NoAI".into(), Tag::Byte(1));
        bat_flags.insert("NoGravity".into(), Tag::Byte(1));
        let bat_id = spawn(game, "minecraft:bat", 4.5, bat_flags);
        let snowball_id = spawn(game, "minecraft:snowball", 3.5, Compound::new());
        let find_entity = |host: &dyn Host, id| host.entity(id);
        let arrow = find_entity(game, arrow_id).unwrap();
        let snowball = find_entity(game, snowball_id).unwrap();
        let bat = find_entity(game, bat_id).unwrap();
        assert!(!bat.leash_to(game, snowball));
        assert!(bat.leash_to(game, arrow));
        assert_eq!(bat.leash_holder(game), Some(LeashHolder::Entity(arrow)));

        let arrow_uuid = arrow.nbt(game).unwrap()[&"UUID".into()].clone();
        let saved_leash = bat.nbt(game).unwrap()[&"leash".into()].clone();
        assert_eq!(saved_leash, Tag::compound([("UUID", arrow_uuid)]));
        assert!(bat.detach_leash(game, false));
        assert!(game.dropped.is_empty());
        assert_eq!(bat.leash_holder(game), Some(LeashHolder::Unleashed));

        assert!(bat.leash_to(game, arrow));
        let arrow_uuid = EntityUuid(arrow.uuid(game).unwrap());
        let bat_uuid = EntityUuid(bat.uuid(game).unwrap());
        let chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert!(game.entity(arrow_id).is_none() && game.entity(bat_id).is_none());
        for (key, tags) in chunks {
            game.world.queue_entity_nbt(key, tags);
        }
        game.spawn_loaded_entities();
        let by_uuid = |host: &dyn ServerHost, uuid| host.entity_by_uuid(uuid).unwrap();
        let arrow = by_uuid(game, arrow_uuid);
        let bat = by_uuid(game, bat_uuid);
        assert_ne!(
            arrow.id(),
            arrow_id,
            "the saved holder was given a new runtime ID"
        );
        assert_ne!(bat.id(), bat_id);
        game.tick_leads();
        assert_eq!(bat.leash_holder(game), Some(LeashHolder::Entity(arrow)));

        game.ticked_entities_through = game.next_entity;
        assert_eq!(bat.velocity(game).unwrap()[0], 0.0);
        game.tick_creatures().unwrap();
        assert!(
            bat.velocity(game).unwrap()[0] > 0.0,
            "the loaded bat is pulled toward the arrow"
        );

        assert!(arrow.discard(game));
        assert_eq!(
            bat.leash_holder(game),
            None,
            "the removed arrow is already inaccessible"
        );
        game.tick_creatures().unwrap();
        assert_eq!(bat.leash_holder(game), Some(LeashHolder::Unleashed));
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:lead")
                .count(),
            1
        );
        game.tick_creatures().unwrap();
        assert_eq!(
            game.dropped
                .iter()
                .filter(|item| item.stack.item.as_ref() == "minecraft:lead")
                .count(),
            1
        );
    });
}
