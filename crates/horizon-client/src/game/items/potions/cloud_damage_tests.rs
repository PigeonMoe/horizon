use super::*;
use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
use horizon_client::{chunk::Chunk, motion::PlayerMotion};
use mod_api::{
    EntityDamageTargetPreEvent, Flow, Hooks, LevelRandomHost, LivingIncomingDamageEvent, ModEntry,
    Mods,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const OWNER: u128 = 41;
const OWNER_ID: i32 = 10;
const VICTIM: u32 = 20;
const CLOUD_AT: Vec3 = Vec3::new(8.5, 64.0, 8.5);

#[derive(Default)]
struct Observation {
    deny: AtomicBool,
    owners: Mutex<Vec<(u128, [f64; 3])>>,
    sources: Mutex<Vec<(i32, i32)>>,
}

struct Guard(Arc<Observation>);

#[mod_api::__subscriptions]
impl Hooks for Guard {
    fn entity_damage_target_pre(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut EntityDamageTargetPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.target_kind, "minecraft:zombie");
        let owner = event.player.as_ref().expect("the cloud's player owner");
        self.0
            .owners
            .lock()
            .unwrap()
            .push((owner.uuid, owner.position));
        if self.0.deny.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn living_incoming_damage(
        &self,
        _: &mut dyn LevelRandomHost,
        event: &mut LivingIncomingDamageEvent<'_>,
    ) -> Flow {
        assert_eq!(event.entity as u32, VICTIM);
        assert_eq!(event.source.damage_type, "minecraft:indirect_magic");
        self.0
            .sources
            .lock()
            .unwrap()
            .push((event.source.causing, event.source.direct));
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_healing_cloud_damaging_undead_keeps_its_live_owner_and_direct_entity() {
    let seen = Arc::new(Observation::default());
    seen.deny.store(true, Ordering::Relaxed);
    let mods: Mods = vec![ModEntry::new("cloud-damage", Box::new(Guard(seen.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|g| {
            g.world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let original = Vec3::new(2.5, 64.0, 2.5);
            let live = Vec3::new(4.5, 64.0, 3.5);
            g.player = Some(PlayerMotion::new(g.motion, original).unwrap());
            bind_local_player(g, OWNER_ID as u32, OWNER);
            let owner = g.potion_owner(Some(Who::Player));
            let zombie = g.res.mob_types()["minecraft:zombie"].clone();
            g.mobs.push(Mob::new(VICTIM, zombie, CLOUD_AT, 0.0));
            let starting_health = g.mobs[0].health;
            let stack =
                ItemStack::new("minecraft:lingering_potion", 1).with_potion("minecraft:healing");
            g.splash(&stack, CLOUD_AT, None, None, 10, &owner, false, Vec3::ZERO);
            g.player.as_mut().unwrap().position = live;

            let mut clouds = g.breath_clouds.lift();
            clouds.update(0, |cloud| {
                cloud.age = 9;
                assert!(g.tick_potion_cloud(cloud, &[]));
                assert_eq!(g.mobs[0].health, starting_health);
                assert!(seen.sources.lock().unwrap().is_empty());

                seen.deny.store(false, Ordering::Relaxed);
                cloud.age = 29;
                assert!(g.tick_potion_cloud(cloud, &[]));
                assert_eq!(g.mobs[0].health, starting_health - 3.0);
                assert_eq!(
                    *seen.owners.lock().unwrap(),
                    [(OWNER, live.components()); 2]
                );
                assert_eq!(*seen.sources.lock().unwrap(), [(OWNER_ID, cloud.id as i32)]);
            });
        });
    });
}
