use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_device_placement_pre(
        &self,
        call: mod_api::__private::DevicePlacementPreCall<'_>,
    ) {
        self.fire_device_placement_pre(call);
    }
}
