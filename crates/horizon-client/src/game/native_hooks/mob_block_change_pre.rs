use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_mob_block_change_pre(
        &self,
        dimension: &str,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
        cancel: &mut bool,
    ) {
        self.fire_mob_block_change_pre(dimension, entity, kind, pos, cancel);
    }

    pub(in crate::game) fn mob_block_change_allowed(
        &self,
        entity: u32,
        kind: &str,
        pos: [i32; 3],
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::MobBlockChangePre)
        {
            return true;
        }
        let mut cancel = false;
        self.native_mob_block_change_pre(self.dim.id(), entity, kind, pos, &mut cancel);
        !cancel
    }
}
