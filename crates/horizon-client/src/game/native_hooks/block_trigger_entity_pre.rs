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

    pub(in crate::game) fn count_allowed_block_triggers(
        &self,
        pos: BlockPos,
        area: Aabb,
        query: EntityQuery,
        mode: BlockTriggerCount,
    ) -> usize {
        let mut count = 0;
        let mut visit = |entity, player: Option<BlockTriggerPlayer<'_>>| {
            let (player, name, permission_level) = player
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
            count += usize::from(!cancel);
            mode == BlockTriggerCount::First || (mode == BlockTriggerCount::Any && !cancel)
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
    use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
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
