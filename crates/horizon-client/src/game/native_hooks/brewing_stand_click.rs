use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_brewing_stand_click(
        &self,
        call: mod_api::__private::BrewingStandClickCall<'_>,
    ) {
        self.fire_brewing_stand_click(call);
    }
}
