use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_entity_damage_pre(
        &mut self,
        call: mod_api::__private::PlayerEntityDamagePreCall<'_>,
    ) {
        self.fire_player_entity_damage_pre(call);
    }
}
