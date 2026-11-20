use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_guest_boat_tick_move_pre(
        &self,
        call: mod_api::__private::GuestBoatTickMovePreCall<'_>,
    ) {
        self.fire_guest_boat_tick_move_pre(call);
    }
}
