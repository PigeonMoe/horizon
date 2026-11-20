use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_arrow_hit_armor_stand_post(
        &mut self,
        call: mod_api::__private::ArrowHitArmorStandPostCall<'_>,
    ) {
        self.fire_arrow_hit_armor_stand_post(call);
    }
}
