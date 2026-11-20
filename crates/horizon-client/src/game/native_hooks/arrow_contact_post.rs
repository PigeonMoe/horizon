use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_arrow_contact_post(
        &mut self,
        call: mod_api::__private::ArrowContactPostCall<'_>,
    ) {
        self.fire_arrow_contact_post(call);
    }
}
