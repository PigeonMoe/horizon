use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_device_break_pre(
        &self,
        call: mod_api::__private::DeviceBreakPreCall<'_>,
    ) {
        self.fire_device_break_pre(call);
    }
}
