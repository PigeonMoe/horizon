//! Block callbacks shared by thrown projectiles and fishing bobbers.
use super::{Game, creatures::ProjectileHitter};
use horizon_client::{Result, arrows::OwnerLookup, mobs::Who};
use horizon_core::{game_mode::GameMode, geometry::Vec3, nbt::Tag, position::BlockPos};

impl Game<'_> {
    /// The common block-state callbacks before projectile-specific breaking.
    pub(super) fn projectile_block_impact(
        &mut self,
        projectile: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) -> Result<()> {
        self.bell_projectile_hit(projectile, pos, at, face);
        self.amethyst_projectile_hit(pos);
        self.projectile_tilts_dripleaf(projectile, pos)
    }

    pub(in crate::game) fn projectile_tilts_dripleaf(
        &mut self,
        shot: &ProjectileHitter,
        pos: [i32; 3],
    ) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if self
            .res
            .behaviours()
            .data(state)
            .is(horizon_world::level::classes::Class::BigDripleaf)
            && self.mob_block_change_allowed(shot.id, shot.kind, pos)
            && self.with_level(|blocks, level, ticks| {
                blocks.big_dripleaf_projectile_hit(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                )
            })
        {
            self.process_level_events()?;
        }
        Ok(())
    }

    /// Paper's `EntityChangeBlockEvent` precedes the mutation and drops.
    pub(in crate::game) fn projectile_destructible_block_hit(
        &mut self,
        shot: &ProjectileHitter,
        owner_uuid: Option<[i32; 4]>,
        can_break: Option<&Tag>,
        hit_speed: f64,
        pos: [i32; 3],
    ) {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = &*self.res.facts().get(state).name;
        let loot_entity = match name {
            "minecraft:chorus_flower" | "minecraft:decorated_pot" => Some(shot.id),
            "minecraft:pointed_dripstone" | "minecraft:sulfur_spike"
                if shot.kind == "minecraft:trident" && hit_speed > 0.6 =>
            {
                None
            }
            _ => return,
        };
        if !self.projectile_may_break(shot, owner_uuid, can_break, pos)
            || !self.mob_block_change_allowed(shot.id, shot.kind, pos)
        {
            return;
        }
        if name == "minecraft:decorated_pot" {
            let registry = &self.pack.assets.registry;
            let Some(mut pot) = registry.state(state).cloned() else {
                return;
            };
            pot.properties.insert("cracked".into(), "true".into());
            let Ok(cracked) = registry.resolve(&pot) else {
                return;
            };
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
        }
        let _ = self.destroy_block_by(pos, loot_entity);
    }

    /// `Projectile.mayInteract` and `mayBreak`, including the live owner's
    /// mode and spawn protection, the entity tag, and the shot's predicate.
    fn projectile_may_break(
        &self,
        shot: &ProjectileHitter,
        owner_uuid: Option<[i32; 4]>,
        can_break: Option<&Tag>,
        pos: [i32; 3],
    ) -> bool {
        use horizon_client::gamerules::vanilla::{MOB_GRIEFING, PROJECTILES_CAN_BREAK_BLOCKS};
        if !self
            .stations()
            .enchantments
            .entity_tags
            .get("minecraft:impact_projectiles")
            .is_some_and(|tag| tag.contains(shot.kind))
            || !self.ui.rules.bool_at(PROJECTILES_CAN_BREAK_BLOCKS)
        {
            return false;
        }
        let has_predicate = || {
            can_break.is_some_and(|predicate| {
                super::game_mode::projectile_predicate_supported(predicate)
                    && self.adventure_predicate_at(predicate, pos)
            })
        };
        let host = self.lan.as_ref();
        let guest = shot.guest.and_then(|conn| {
            let host = host?;
            let guest = host.hub.guests.iter().find(|g| g.conn == conn)?;
            Some((host, guest))
        });
        if shot.owner == Some(Who::Player) && (shot.guest.is_none() || guest.is_some()) {
            let (mode, protected) = guest.map_or((self.mode.id(), false), |(host, guest)| {
                let protected = self
                    .spawn_protection_for(host, guest.uuid)
                    .is_some_and(|radius| self.spawn_protects(radius, self.dim.id(), pos));
                (guest.game_mode, protected)
            });
            return !protected
                && self.border.geometry().is_within_bounds(
                    f64::from(pos[0]),
                    f64::from(pos[2]),
                    0.0,
                )
                && (mode != GameMode::Adventure.id() || has_predicate());
        }
        if matches!(shot.owner, Some(Who::Mob(_)))
            || owner_uuid.is_some_and(|id| self.non_player_entity(id))
        {
            return self.ui.rules.bool_at(MOB_GRIEFING) && has_predicate();
        }
        let default_mode = host.map_or_else(
            || self.level.mode(),
            |host| host.hub.config.default_game_mode,
        );
        default_mode != GameMode::Adventure.id() || has_predicate()
    }

    /// `TargetBlock.onProjectileHit`: a target block takes its power from
    /// where the projectile landed and resets after 20 ticks for an arrow, 8
    /// for any other projectile. A player's shot counts as a target hit.
    pub(super) fn target_block_hit(
        &mut self,
        p: &ProjectileHitter,
        pos: [i32; 3],
        at: Vec3,
        face: [i32; 3],
    ) {
        let block = BlockPos::new(pos[0], pos[1], pos[2]);
        let arrow = p.arrow;
        let location = [at.x, at.y, at.z];
        let hit = self.with_level(|b, level, ticks| {
            b.target_projectile_hit(level, ticks, block, location, face, arrow)
        });
        if let Some(strength) = hit {
            let _ = self.process_level_events();
            self.native_target_hit(self.dim.id(), pos, strength);
            if p.owner == Some(Who::Player) && (p.guest.is_none() || p.guest == self.lan_acting) {
                self.award_custom_stat("target_hit", 1);
                let projectile = horizon_client::advancements::EntityView {
                    kind: p.kind.to_owned(),
                    position: [p.position.x, p.position.y, p.position.z],
                    ..Default::default()
                };
                self.trigger_target_hit(&projectile, strength);
            }
        }
    }
}
