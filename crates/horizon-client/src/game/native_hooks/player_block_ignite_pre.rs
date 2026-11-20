use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_player_block_ignite_pre(
        &self,
        call: mod_api::__private::PlayerBlockIgnitePreCall<'_>,
    ) {
        self.fire_player_block_ignite_pre(call);
    }
}
