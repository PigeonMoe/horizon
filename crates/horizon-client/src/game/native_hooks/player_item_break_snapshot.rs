use super::Game;

impl Game<'_> {
    pub(in crate::game) fn native_player_item_break_snapshot(
        &mut self,
        call: mod_api::__private::PlayerItemBreakSnapshotCall<'_>,
    ) {
        self.fire_player_item_break_snapshot(call);
    }
}
