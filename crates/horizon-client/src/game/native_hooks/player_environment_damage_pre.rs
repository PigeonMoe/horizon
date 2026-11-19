use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_environment_damage_pre(
        &mut self,
        call: mod_api::__private::PlayerEnvironmentDamagePreCall<'_>,
    ) {
        self.fire_player_environment_damage_pre(call);
    }
}
