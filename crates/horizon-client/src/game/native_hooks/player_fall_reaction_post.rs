use super::*;
use crate::game::commands::Target;
use horizon_client::{landing::FallEvent, motion::MotionEvent};
use horizon_core::geometry::Vec3;
use horizon_core::random::RandomSource;

impl Game<'_> {
    pub(in crate::game) fn native_player_fall_reaction_post(
        &mut self,
        call: mod_api::__private::PlayerFallReactionPostCall,
    ) {
        self.fire_player_fall_reaction_post(call);
    }

    pub(in crate::game) fn apply_fall_reaction(&mut self, event: MotionEvent) {
        let MotionEvent::Fall(FallEvent::EquipmentEffect {
            player,
            reaction,
            origin,
            bounds,
            eye,
            damage,
            sound,
        }) = event
        else {
            return;
        };
        let Some(player) = self.player(player) else {
            return;
        };
        if let Some((name, pitch)) = sound {
            let seed = self.world.random.next_long();
            self.play_sound_to(
                Target::Player,
                name,
                eye,
                1.0,
                pitch,
                mod_api::SoundSource::Players.index(),
                seed,
            );
        }
        let [x, y, z] = origin;
        let at = Vec3::new(x, y, z);
        let previous = self
            .player
            .as_mut()
            .map(|body| std::mem::replace(&mut body.position, at));
        // Permission listeners see the attacker's position at impact even when
        // the network delivered more than one move in this server tick.
        self.native_player_fall_reaction_post(mod_api::__private::PlayerFallReactionPostCall {
            player,
            reaction,
            origin,
            bounds,
            damage,
        });
        if let (Some(body), Some(previous)) = (self.player.as_mut(), previous)
            && body.position == at
        {
            body.position = previous;
        }
    }
}
