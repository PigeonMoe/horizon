use super::*;
use std::collections::HashSet;

impl Game<'_> {
    /// `EnderDragon.checkWalls` over the head, neck and body. A block that is
    /// neither air nor `#dragon_transparent` breaks when griefing is allowed
    /// (`canEntityGrief` for the dragon), its parts let the dragon destroy it
    /// (unanswered: not `#dragon_immune`), no mod keeps it
    /// (`LivingDestroyBlockEvent`) and it is not `#dragon_immune`, which a yes
    /// does not bypass; any other puts the dragon in a wall.
    pub(super) fn dragon_check_walls(&mut self) {
        let Some(d) = self.dragon.as_ref() else {
            return;
        };
        let griefing = self.can_entity_grief(Some(d.id));
        let boxes = [d.part_box(0), d.part_box(1), d.part_box(2)];
        let dragon_id = d.id;
        let center = [d.position.x, d.position.y, d.position.z];
        let tags = &self.items().block_tags;
        let transparent = tags
            .get("minecraft:dragon_transparent")
            .cloned()
            .unwrap_or_default();
        let immune = tags
            .get("minecraft:dragon_immune")
            .cloned()
            .unwrap_or_default();
        let mut in_wall = false;
        let mut broke = false;
        for bounds in boxes {
            let (part_in_wall, part_broke) = self.dragon_check_wall_part(
                bounds,
                dragon_id,
                center,
                griefing,
                &transparent,
                &immune,
            );
            in_wall |= part_in_wall;
            broke |= part_broke;
        }
        if broke && let Some(d) = self.dragon.as_ref() {
            let p = d.parts[0].0;
            // Level event 2008: an explosion puff where a block broke.
            self.particles.add(
                &ParticleOptions::simple(ParticleType::Explosion),
                [p.x, p.y, p.z],
                [0.0; 3],
            );
        }
        if let Some(d) = self.dragon.as_mut() {
            d.in_wall = in_wall;
        }
    }

    fn dragon_check_wall_part(
        &mut self,
        bounds: Aabb,
        dragon_id: u32,
        center: [f64; 3],
        griefing: bool,
        transparent: &HashSet<String>,
        immune: &HashSet<String>,
    ) -> (bool, bool) {
        let mut in_wall = false;
        let mut removed = Vec::new();
        let (lo, hi) = (bounds.min_position(), bounds.max_position());
        for x in lo.x.floor() as i32..=hi.x.floor() as i32 {
            for y in lo.y.floor() as i32..=hi.y.floor() as i32 {
                for z in lo.z.floor() as i32..=hi.z.floor() as i32 {
                    let state = self.world.block(x, y, z);
                    let name = &self.res.facts().get(state).name;
                    if self.res.facts().is_air(state) || transparent.contains(&**name) {
                        continue;
                    }
                    // Griefing, then what the block's parts answer.
                    let is_immune = immune.contains(&**name);
                    if griefing && self.entity_may_destroy(dragon_id, [x, y, z], !is_immune) {
                        removed.push(([x, y, z], is_immune));
                    } else {
                        in_wall = true;
                    }
                }
            }
        }
        // LivingDestroyBlockEvent: a block a mod keeps is still in the wall.
        if !mod_api::__private::mods().is_empty() {
            removed.retain(|&(p, _)| {
                let kept = !self.may_destroy(dragon_id, "minecraft:ender_dragon", p);
                in_wall |= kept;
                !kept
            });
        }
        // #dragon_immune still holds after both: such a block stays, in the wall.
        removed.retain(|&(_, is_immune)| {
            in_wall |= is_immune;
            !is_immune
        });
        if removed.is_empty() {
            return (in_wall, false);
        }
        let mut blocks: Vec<_> = removed.into_iter().map(|(pos, _)| pos).collect();
        // Paper raises a block-only EntityExplodeEvent for each occupied part.
        // A veto keeps this batch without changing the vanilla in-wall result.
        if self.native_authoritative()
            && mod_api::__private::listened(mod_api::HookId::ExplosionDetonate)
        {
            self.filter_explosion_targets(center, &mut blocks);
        }
        let mut broke = false;
        for [x, y, z] in blocks {
            broke |= self.with_level(|behaviours, level, ticks| {
                behaviours.remove_block(level, ticks, BlockPos::new(x, y, z), false)
            });
            let _ = self.process_level_events();
        }
        (in_wall, broke)
    }
}
