use super::*;
use crate::game::native_entity_refs::tests::{run_on_guest_stack, with_game};
use horizon_core::geometry::Vec3;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use mod_api::{Flow, Hooks, Host, MobBlockChangePreEvent, ModEntry};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "golem_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

struct StatueProtection(Arc<AtomicUsize>);

#[mod_api::__subscriptions]
impl Hooks for StatueProtection {
    fn mob_block_change_pre(&self, _: &dyn Host, event: &mut MobBlockChangePreEvent<'_>) -> Flow {
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.entity, 71);
        assert_eq!(event.kind, "minecraft:copper_golem");
        assert_eq!(event.pos, [0, 70, 0]);
        if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn statue_conversion_keeps_the_owner_until_physical_discard() {
    run_on_guest_stack(|| {
        let calls = Arc::new(AtomicUsize::new(0));
        let mods = [ModEntry::new(
            "statue-protection",
            Box::new(StatueProtection(calls.clone())),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                let kind = game
                    .res
                    .mob_types()
                    .get("minecraft:copper_golem")
                    .unwrap()
                    .clone();
                game.mobs
                    .push(Mob::new(71, kind, Vec3::new(0.5, 70.0, 0.5), 0.0));
                let index = Arc::new(Mutex::new(AttachmentIndex::default()));
                let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71);
                let mut values = Attachments::default();
                VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
                let original = game.mobs.update(0, |mob| {
                    mob.attachment_lifetime().bind(&mut values, &index, holder);
                    mob.attachment_lifetime().registration().unwrap()
                });
                let key = ("test".into(), "golem_lifetime".into());
                let before = index.lock().unwrap().snapshot(&key);
                assert_eq!(before.len(), 1);
                game.mobs.update(0, |mob| {
                    set_held(mob, Some(ItemStack::new("minecraft:stone", 2)))
                });
                let dropped_before = game.dropped.len();
                game.golem_become_statue(71);
                assert_eq!(calls.load(Ordering::Relaxed), 1);
                assert!(game.mobs[0].alive());
                assert_eq!(held(&game.mobs[0]).unwrap().count, 2);
                assert_eq!(game.dropped.len(), dropped_before);
                assert_eq!(game.world.block(0, 70, 0), game.world.air);
                assert!(!game.world.block_entities.contains_key(&(0, 70, 0)));

                game.golem_become_statue(71);
                assert_eq!(calls.load(Ordering::Relaxed), 2);
                assert!(game.mobs[0].removed);
                assert!(matches!(
                    game.world.block_entities.get(&(0, 70, 0)),
                    Some(BlockEntity::CopperGolemStatue(_))
                ));
                assert_eq!(
                    &*game.res.facts().get(game.world.block(0, 70, 0)).name,
                    "minecraft:oxidized_copper_golem_statue"
                );
                assert_eq!(game.dropped.len(), dropped_before + 1);
                assert_eq!(game.dropped.last().unwrap().stack.count, 2);
                let current = game
                    .mobs
                    .update(0, |mob| mob.attachment_lifetime().registration().unwrap());
                assert!(original.same_registration(&current));
                assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
                VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
                let after = index.lock().unwrap().snapshot(&key);
                assert_eq!(after.len(), 1);
                assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
                game.mobs.retain(|mob| !mob.removed);
                assert!(index.lock().unwrap().snapshot(&key).is_empty());
                assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
                assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
            })
        });
    });
}
