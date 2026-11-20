use super::*;
use horizon_client::landing::LandingReaction;

impl Game<'_> {
    pub(in crate::game) fn native_player_environment_damage_pre(
        &mut self,
        call: mod_api::__private::PlayerEnvironmentDamagePreCall<'_>,
    ) {
        self.fire_player_environment_damage_pre(call);
    }

    pub(in crate::game) fn player_environment_damage_cancels(
        &mut self,
        damage_type: &str,
        damage: f32,
    ) -> bool {
        let mut cancel = false;
        if let Some(player) = self.native_acting_player() {
            self.native_player_environment_damage_pre(
                mod_api::__private::PlayerEnvironmentDamagePreCall {
                    player,
                    damage_type,
                    damage,
                    cancel: &mut cancel,
                },
            );
        }
        cancel
    }

    pub(in crate::game) fn direct_fall_footwear_effect(&mut self, reaction: LandingReaction) {
        let Some(body) = self.player.as_ref() else {
            return;
        };
        let at = body.position;
        if let LandingReaction::Footwear(Some(sound)) = reaction {
            let at = [at.x, at.y, at.z];
            self.audio.play_unshared(sound, Some(at), 1.0, 1.0);
            self.send_level_sound(
                crate::game::level_sounds::LevelSound::new(sound, at, 1.0, 1.0)
                    .in_source(mod_api::SoundSource::Players),
                None,
            );
        } else if let Some(event) = reaction.effect_at(at, body.dimensions) {
            self.apply_fall_reaction(event);
        }
    }

    /// Resolve the sole pending landing with the real player and their inventory installed.
    pub(in crate::game) fn resolve_player_fall_damage(
        &mut self,
        fall: horizon_client::landing::PausedFallDamage,
    ) {
        let before = self.player.as_ref().map(|body| body.events.len());
        // A guest's movement batch can already have put their body at its last packet.
        let previous = self
            .player
            .as_mut()
            .map(|body| std::mem::replace(&mut body.position, fall.position));
        let cancel = self.player_environment_damage_cancels("minecraft:fall", fall.damage);
        if let (Some(body), Some(previous)) = (self.player.as_mut(), previous)
            && body.position == fall.position
        {
            body.position = previous;
        }
        let rules = self.world.health_rules;
        if !cancel && let Some(player) = self.player.as_mut() {
            let _ = player.finish_hurt(
                crate::game::entity::damage_level_of!(self),
                &self.health_resources,
                rules,
                fall.hit,
            );
        }
        // Preserve the position of the hurt events among this move's landing events.
        if let (Some(body), Some(before)) = (self.player.as_mut(), before)
            && fall.event_index <= before
            && before <= body.events.len()
        {
            body.events[fall.event_index..].rotate_left(before - fall.event_index);
        }
    }
}
