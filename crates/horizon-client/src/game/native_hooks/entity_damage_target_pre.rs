use super::*;
use crate::game::commands::Target;

impl HitLevel<'_> {
    pub(in crate::game) fn native_entity_damage_target_pre(
        &mut self,
        dimension: &str,
        target_kind: &str,
        position: [f64; 3],
        alive_kind: bool,
        player: Option<&mod_api::DamagePlayer>,
        cancel: &mut bool,
    ) {
        self.fire_entity_damage_target_pre(
            dimension,
            target_kind,
            position,
            alive_kind,
            player,
            cancel,
        );
    }
}

impl Game<'_> {
    /// Snapshot the current online player's world and permissions. A guest
    /// swapped in for a tick must be read from the active player body.
    pub(in crate::game) fn damage_player_by_uuid(
        &self,
        uuid: u128,
    ) -> Option<mod_api::DamagePlayer> {
        let active_uuid = self.lan_acting_uuid.or_else(|| {
            self.level
                .player_uuid
                .map(horizon_client::lan::playerdata::ints_uuid)
        });
        if active_uuid == Some(uuid)
            && let Some(body) = self.player.as_ref()
            && !self.lan_acting_fake
        {
            return Some(mod_api::DamagePlayer {
                uuid,
                name: self.acting_player_name().to_owned(),
                permission: i32::from(
                    self.lan_acting_permission
                        .unwrap_or(self.ui.registries.permission),
                ),
                dimension: self.dim.id().to_owned(),
                position: [body.position.x, body.position.y, body.position.z],
            });
        }
        let guest = self
            .lan
            .as_ref()?
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == uuid && g.placed && !g.is_fake())?;
        Some(mod_api::DamagePlayer {
            uuid,
            name: guest.name.clone(),
            permission: i32::from(guest.permission),
            dimension: guest.dimension.clone(),
            position: guest.position,
        })
    }

    pub(in crate::game) fn damage_player_by_entity(
        &self,
        id: i32,
    ) -> Option<mod_api::DamagePlayer> {
        if id == 0
            || !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityDamageTargetPre)
        {
            return None;
        }
        self.damage_player_from_attacker(id)
    }

    pub(super) fn damage_player_from_attacker(&self, id: i32) -> Option<mod_api::DamagePlayer> {
        if id == 0 {
            return None;
        }
        let int_uuid = horizon_client::lan::playerdata::ints_uuid;
        let uuid = if self
            .player
            .as_ref()
            .is_some_and(|body| body.entity_id == id)
        {
            self.lan_acting_uuid
                .or_else(|| self.level.player_uuid.map(int_uuid))
        } else if let Some(g) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guests.iter().find(|g| g.entity_id == id))
        {
            Some(g.uuid)
        } else {
            self.mob_object(u32::try_from(id).ok()?)
                .filter(|mob| mob.extra.tame)
                .and_then(|mob| mob.extra.owner)
                .map(int_uuid)
        }?;
        self.damage_player_by_uuid(uuid)
    }

    pub(in crate::game) fn damage_player_from_target(
        &self,
        source: Target,
    ) -> Option<mod_api::DamagePlayer> {
        let attacker = match source {
            Target::Player => self.own_entity_id(),
            Target::Guest(conn) => self.command_guest(conn)?.entity_id,
            Target::Mob(id) => id as i32,
            _ => return None,
        };
        self.damage_player_from_attacker(attacker)
    }

    pub(in crate::game) fn entity_target_damage_allowed(
        &mut self,
        kind: &str,
        position: [f64; 3],
        living: bool,
        player: Option<&mod_api::DamagePlayer>,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityDamageTargetPre)
        {
            return true;
        }
        let dimension = self.dim.id();
        let mut cancel = false;
        let level = crate::game::entity::damage_level_of!(self);
        HitLevel(level.reborrow()).native_entity_damage_target_pre(
            dimension,
            kind,
            position,
            living,
            player,
            &mut cancel,
        );
        !cancel
    }
}
