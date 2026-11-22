use super::Game;

impl Game<'_> {
    /// Dispatch immutable validation of a required section to its configured owner.
    pub(in crate::game) fn native_player_transfer_profile_stage(
        &self,
        call: mod_api::__private::PlayerTransferProfileStageCall<'_>,
    ) {
        self.fire_player_transfer_profile_stage(call);
    }
}
