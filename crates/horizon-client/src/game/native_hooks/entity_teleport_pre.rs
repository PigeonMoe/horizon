use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_entity_teleport_pre(
        &self,
        call: mod_api::__private::EntityTeleportPreCall<'_>,
    ) {
        self.fire_entity_teleport_pre(call);
    }
}
