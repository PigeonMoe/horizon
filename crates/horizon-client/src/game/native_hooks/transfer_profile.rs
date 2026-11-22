use super::Game;

impl Game<'_> {
    /// Dispatch a source snapshot to the required section's package only.
    pub(in crate::game) fn native_player_transfer_profile_capture(
        &self,
        call: mod_api::__private::PlayerTransferProfileCaptureCall<'_>,
    ) {
        self.fire_player_transfer_profile_capture(call);
    }

    /// Dispatch immutable validation of a required section to its configured owner.
    pub(in crate::game) fn native_player_transfer_profile_stage(
        &self,
        call: mod_api::__private::PlayerTransferProfileStageCall<'_>,
    ) {
        self.fire_player_transfer_profile_stage(call);
    }
}
