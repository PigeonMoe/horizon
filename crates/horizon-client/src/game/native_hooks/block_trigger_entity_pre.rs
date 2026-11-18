use crate::game::{
    BlockTriggerPlayer, GameLevel,
    entity::{Entity as _, for_each_entity},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use horizon_world::level::{BlockTriggerCount, EntityQuery};

impl GameLevel<'_> {
    pub(in crate::game) fn native_block_trigger_entity_pre(
        &self,
        call: mod_api::__private::BlockTriggerEntityPreCall<'_>,
    ) {
        self.fire_block_trigger_entity_pre(call);
    }

    fn block_trigger_allowed(
        &self,
        pos: BlockPos,
        entity: u32,
        source: Option<BlockTriggerPlayer<'_>>,
    ) -> bool {
        let (player, name, permission_level) = source
            .filter(|source| source.uuid.is_some())
            .map_or((None, "", 0), |source| {
                (source.uuid, source.name, source.permission_level)
            });
        let mut cancel = false;
        self.native_block_trigger_entity_pre(mod_api::__private::BlockTriggerEntityPreCall {
            dimension: self.dimension,
            pos,
            entity,
            player,
            name,
            permission_level,
            cancel: &mut cancel,
        });
        !cancel
    }

    pub(in crate::game) fn allow_known_block_trigger(&self, pos: BlockPos, entity: u32) -> bool {
        let source = self
            .block_trigger_player
            .filter(|source| source.entity == entity)
            .or_else(|| {
                let hub = self.lan_hub?;
                let guest = hub.guests.simulation_iter().find(|guest| {
                    guest.entity_id as u32 == entity
                        && hub.here(guest)
                        && guest.loaded
                        && !guest.dead
                        && !guest.spectator()
                })?;
                Some(BlockTriggerPlayer {
                    entity,
                    uuid: Some(guest.uuid),
                    name: &guest.name,
                    permission_level: guest.permission,
                })
            });
        self.block_trigger_allowed(pos, entity, source)
    }

    pub(in crate::game) fn count_allowed_block_triggers(
        &self,
        pos: BlockPos,
        area: Aabb,
        query: EntityQuery,
        mode: BlockTriggerCount,
    ) -> usize {
        let mut count = 0;
        let mut visit = |entity, player: Option<BlockTriggerPlayer<'_>>| {
            let allowed = self.block_trigger_allowed(pos, entity, player);
            count += usize::from(allowed);
            mode == BlockTriggerCount::First || (mode == BlockTriggerCount::Any && allowed)
        };
        let hit = |(min, max): ([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(min[0], min[1], min[2]),
                Vec3::new(max[0], max[1], max[2]),
            ))
        };
        match query {
            EntityQuery::Arrows => {
                for &(id, bounds) in &self.arrow_boxes {
                    if area.intersects(bounds) && visit(id, None) {
                        break;
                    }
                }
                return count;
            }
            EntityQuery::Minecart => {
                for (&bounds, cart) in self.cart_boxes.iter().zip(self.carts.iter()) {
                    if hit(bounds) && visit(cart.id, None) {
                        break;
                    }
                }
                return count;
            }
            EntityQuery::Everything | EntityQuery::Mobs => {}
        }
        if let Some(player) = self.block_trigger_player
            && self.player_box.is_some_and(hit)
            && visit(player.entity, Some(player))
        {
            return count;
        }
        if let Some(hub) = self.lan_hub {
            for guest in hub.guests.simulation_iter().filter(|guest| {
                hub.here(guest)
                    && guest.loaded
                    && !guest.dead
                    && !guest.spectator()
                    && !self
                        .block_trigger_player
                        .is_some_and(|player| player.entity == guest.entity_id as u32)
            }) {
                let entity = guest.entity_id as u32;
                if area.intersects(guest.bounds())
                    && visit(
                        entity,
                        Some(BlockTriggerPlayer {
                            entity,
                            uuid: Some(guest.uuid),
                            name: &guest.name,
                            permission_level: guest.permission,
                        }),
                    )
                {
                    return count;
                }
            }
        }
        for_each_entity!(self.entities(), entity => {
            if (query != EntityQuery::Mobs || entity.is_living())
                && entity.triggers_blocks()
                && area.intersects(entity.bounds())
                && visit(entity.id(), None)
            {
                return count;
            }
        });
        if query == EntityQuery::Everything {
            let extras = self
                .crystals
                .iter()
                .map(|crystal| (crystal.id, crystal.bounds()))
                .chain(
                    self.trigger_hooks
                        .iter()
                        .map(|&(id, bounds, _)| (id, bounds)),
                )
                .chain(self.trigger_extras.iter().copied());
            for (id, bounds) in extras {
                if area.intersects(bounds) && visit(id, None) {
                    break;
                }
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::{bind_acting_guest, bind_local_player, with_game};
    use horizon_client::{
        chunk::Chunk,
        mobs::Mob,
        motion::PlayerMotion,
        projectiles::{Projectile, ProjectileKind},
    };
    use mod_api::{BlockTriggerEntityPreEvent, Flow, Hooks, LevelHost, ModEntry, Mods};
    use std::sync::{Arc, Mutex};

    use super::*;

    struct Guard(Arc<Mutex<Vec<(u32, Option<u128>)>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Guard {
        fn block_trigger_entity_pre(
            &self,
            _: &dyn LevelHost,
            event: &mut BlockTriggerEntityPreEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push((event.entity, event.player));
            if event.player.is_some() {
                assert_eq!((event.name, event.permission_level), ("rider", 2));
            }
            if event.player.is_some() || event.entity == 10 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn a_denied_standing_entity_does_not_tilt_or_hide_an_allowed_one() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "trigger-guard",
            Box::new(Guard(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let feet = Vec3::new(8.5, 64.9375, 8.5);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let mut player = PlayerMotion::new(game.motion, feet).unwrap();
                player.ground.on_ground = true;
                game.player = Some(player);
                bind_local_player(game, 7, 17);
                game.lan_acting_name = Some("rider".into());
                game.lan_acting_permission = Some(2);
                game.update_chunk_tickets();
                let registry = game.pack.assets.registry;
                let clay = registry.default_state("minecraft:clay").unwrap();
                let leaf = registry.default_state("minecraft:big_dripleaf").unwrap();
                game.world.set(8, 63, 8, clay).unwrap();
                game.world.set(8, 64, 8, leaf).unwrap();
                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                let mut denied = Mob::new(10, kind.clone(), feet, 0.0);
                denied.on_ground = true;
                game.mobs.push(denied);

                game.tick_blocks().unwrap();
                assert_eq!(game.world.block(8, 64, 8), leaf);
                assert_eq!(
                    std::mem::take(&mut *heard.lock().unwrap()),
                    [(7, Some(17)), (10, None)]
                );

                let mut allowed = Mob::new(11, kind, feet, 0.0);
                allowed.on_ground = true;
                game.mobs.push(allowed);
                game.tick_blocks().unwrap();
                let state = registry.state(game.world.block(8, 64, 8)).unwrap();
                assert_eq!(state.properties["tilt"], "unstable");
                assert_eq!(
                    std::mem::take(&mut *heard.lock().unwrap()),
                    [(7, Some(17)), (10, None), (11, None)]
                );

                game.lan_acting = Some(8);
                game.lan_acting_entity = Some(12);
                game.lan_acting_uuid = Some(22);
                bind_acting_guest(game);
                game.lan_acting = None;
                game.lan_acting_entity = None;
                game.lan_acting_uuid = None;
                game.player.as_mut().unwrap().position = Vec3::new(9.5, 65.0, 8.5);
                game.mobs.clear();
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.loaded = true;
                    guest.on_ground = true;
                    guest.permission = 2;
                });
                game.world.set(8, 64, 8, leaf).unwrap();
                game.tick_blocks().unwrap();
                assert_eq!(game.world.block(8, 64, 8), leaf);
                assert_eq!(*heard.lock().unwrap(), [(12, Some(22))]);
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn redstone_activation_checks_real_players_and_other_entities_per_trigger() {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "trigger-guard",
            Box::new(Guard(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let at = BlockPos::new(8, 64, 8);
                let feet = Vec3::new(8.5, 64.0, 8.5);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player = Some(PlayerMotion::new(game.motion, feet).unwrap());
                bind_local_player(game, 7, 17);
                game.lan_acting_name = Some("rider".into());
                game.lan_acting_permission = Some(2);
                let registry = game.pack.assets.registry;
                let stone = registry.default_state("minecraft:stone").unwrap();
                game.world.set(8, 63, 8, stone).unwrap();
                let activate = |game: &mut crate::game::Game<'_>| {
                    game.with_level(|blocks, level, ticks| blocks.entity_inside(level, ticks, at));
                };
                let state = |game: &crate::game::Game<'_>, property: &str| {
                    registry
                        .state(game.world.block(8, 64, 8))
                        .unwrap()
                        .properties[property]
                        .clone()
                };
                let plain = registry
                    .default_state("minecraft:stone_pressure_plate")
                    .unwrap();
                game.world.set(8, 64, 8, plain).unwrap();
                activate(game);
                assert_eq!(state(game, "powered"), "false");
                assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), [(7, Some(17))]);

                let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
                game.mobs.push(Mob::new(8, kind, feet, 0.0));
                for (block, prop, expected) in [
                    ("minecraft:stone_pressure_plate", "powered", "true"),
                    ("minecraft:light_weighted_pressure_plate", "power", "1"),
                    ("minecraft:tripwire", "powered", "true"),
                ] {
                    let mut candidate = registry.default_state(block).unwrap();
                    if block == "minecraft:tripwire" {
                        candidate = crate::game::native_hooks::with_property(
                            registry, candidate, "attached", "true",
                        )
                        .unwrap();
                    }
                    game.world.set(8, 64, 8, candidate).unwrap();
                    activate(game);
                    assert_eq!(state(game, prop), expected, "{block}");
                    assert_eq!(
                        std::mem::take(&mut *heard.lock().unwrap()),
                        [(7, Some(17)), (8, None)]
                    );
                }

                let button = crate::game::native_hooks::with_property(
                    registry,
                    registry.default_state("minecraft:oak_button").unwrap(),
                    "face",
                    "floor",
                )
                .unwrap();
                game.world.set(8, 64, 8, button).unwrap();
                for id in [10, 11] {
                    let mut projectile = Projectile::new(
                        ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        },
                        Vec3::new(8.5, 64.1, 8.5),
                        None,
                    );
                    projectile.id = id;
                    game.creatures.projectiles.push(projectile);
                }
                activate(game);
                assert_eq!(state(game, "powered"), "false");
                assert_eq!(std::mem::take(&mut *heard.lock().unwrap()), [(10, None)]);
            });
        });
    }
}
