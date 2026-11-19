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
    owners: Mutex<Vec<Option<(u128, [f64; 3])>>>,
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
        let owner = event.player.map(|owner| (owner.uuid, owner.position));
        self.0.owners.lock().unwrap().push(owner);
        if owner.is_some() && self.0.deny.load(Ordering::Relaxed) {
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
            g.splash(
                &stack,
                CLOUD_AT,
                None,
                None,
                10,
                &owner,
                false,
                (None, Vec3::ZERO),
            );
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
                    [Some((OWNER, live.components())); 2]
                );
                assert_eq!(*seen.sources.lock().unwrap(), [(OWNER_ID, cloud.id as i32)]);
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_saved_healing_cloud_resolves_its_online_owner_after_reload() {
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
            let stack =
                ItemStack::new("minecraft:lingering_potion", 1).with_potion("minecraft:healing");
            g.splash(
                &stack,
                CLOUD_AT,
                None,
                None,
                10,
                &owner,
                false,
                (None, Vec3::ZERO),
            );
            let uuid = horizon_client::lan::playerdata::uuid_ints(OWNER);
            let saved = potion_cloud_to_nbt(&g.breath_clouds[0], [4, 3, 2, 1], None);
            assert_eq!(saved.get("Owner"), Some(&Tag::IntArray(uuid.to_vec())));
            g.breath_clouds.clear();
            let cloud_id = g.load_potion_cloud(&saved).unwrap();
            // Leaving keeps the saved identity but supplies no live attacker.
            g.player = None;
            let kind = g.res.mob_types()["minecraft:zombie"].clone();
            g.mobs.push(Mob::new(VICTIM, kind.clone(), CLOUD_AT, 0.0));
            let starting_health = g.mobs[0].health;
            let mut clouds = g.breath_clouds.lift();
            clouds.update(0, |cloud| {
                cloud.age = 9;
                assert!(g.tick_potion_cloud(cloud, &[]));
                assert_eq!(g.mobs[0].health, starting_health - 3.0);
                assert_eq!(*seen.owners.lock().unwrap(), [None]);
                assert_eq!(*seen.sources.lock().unwrap(), [(0, cloud_id as i32)]);
                assert_eq!(
                    potion_cloud_to_nbt(cloud, [4, 3, 2, 1], None).get("Owner"),
                    Some(&Tag::IntArray(uuid.to_vec()))
                );

                // Reconnect with a new runtime id; the same cloud resolves the player.
                const RECONNECTED_ID: u32 = 35;
                g.player = Some(PlayerMotion::new(g.motion, live).unwrap());
                bind_local_player(g, RECONNECTED_ID, OWNER);
                g.mobs.clear();
                g.mobs.push(Mob::new(VICTIM, kind, CLOUD_AT, 0.0));
                cloud.age = 29;
                assert!(g.tick_potion_cloud(cloud, &[]));
                assert_eq!(g.mobs[0].health, starting_health);

                seen.deny.store(false, Ordering::Relaxed);
                cloud.age = 49;
                assert!(g.tick_potion_cloud(cloud, &[]));
                assert_eq!(g.mobs[0].health, starting_health - 3.0);
                assert_eq!(
                    *seen.owners.lock().unwrap(),
                    [
                        None,
                        Some((OWNER, live.components())),
                        Some((OWNER, live.components()))
                    ]
                );
                assert_eq!(
                    *seen.sources.lock().unwrap(),
                    [
                        (0, cloud_id as i32),
                        (RECONNECTED_ID as i32, cloud_id as i32)
                    ]
                );
            });
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_splash_healing_potion_uses_its_direct_entity_when_it_hurts_undead() {
    let seen = Arc::new(Observation::default());
    seen.deny.store(true, Ordering::Relaxed);
    let mods: Mods = vec![ModEntry::new(
        "splash-damage",
        Box::new(Guard(seen.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|g| {
            const PROJECTILE: u32 = 25;
            g.world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let live = Vec3::new(2.5, 64.0, 2.5);
            g.player = Some(PlayerMotion::new(g.motion, live).unwrap());
            bind_local_player(g, OWNER_ID as u32, OWNER);
            let owner = g.potion_owner(Some(Who::Player));
            let kind = g.res.mob_types()["minecraft:zombie"].clone();
            g.mobs.push(Mob::new(VICTIM, kind, CLOUD_AT, 0.0));
            let starting_health = g.mobs[0].health;
            let stack =
                ItemStack::new("minecraft:splash_potion", 1).with_potion("minecraft:healing");
            for denied in [true, false] {
                seen.deny.store(denied, Ordering::Relaxed);
                g.splash(
                    &stack,
                    CLOUD_AT,
                    Some(CLOUD_AT),
                    None,
                    10,
                    &owner,
                    false,
                    (Some(PROJECTILE), Vec3::ZERO),
                );
                assert_eq!(
                    g.mobs[0].health,
                    starting_health - if denied { 0.0 } else { 6.0 }
                );
            }
            assert_eq!(
                *seen.owners.lock().unwrap(),
                [Some((OWNER, live.components())); 2]
            );
            assert_eq!(
                *seen.sources.lock().unwrap(),
                [(OWNER_ID, PROJECTILE as i32)]
            );
        });
    });
}
