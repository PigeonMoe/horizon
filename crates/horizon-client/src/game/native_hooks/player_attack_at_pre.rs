use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_attack_at_pre(
        &self,
        call: mod_api::__private::PlayerAttackAtPreCall<'_>,
    ) {
        self.fire_player_attack_at_pre(call);
    }
}
