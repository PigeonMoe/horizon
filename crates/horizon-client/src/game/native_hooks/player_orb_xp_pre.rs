use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_orb_xp_pre(
        &mut self,
        call: mod_api::__private::PlayerOrbXpPreCall<'_>,
    ) {
        self.fire_player_orb_xp_pre(call);
    }
}
