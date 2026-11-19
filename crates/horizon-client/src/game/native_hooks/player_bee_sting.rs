use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_bee_sting(
        &mut self,
        call: mod_api::__private::PlayerBeeStingCall<'_>,
    ) {
        self.fire_player_bee_sting(call);
    }
}
