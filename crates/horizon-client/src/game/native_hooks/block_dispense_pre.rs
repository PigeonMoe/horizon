use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_dispenser_selected(
        &mut self,
        call: mod_api::__private::DispenserSelectedCall<'_>,
    ) {
        self.fire_dispenser_selected(call);
    }

    pub(in crate::game) fn native_block_dispense_pre(
        &self,
        dimension: &str,
        pos: [i32; 3],
        target: [i32; 3],
        item: &str,
        cancel: &mut bool,
    ) {
        self.fire_block_dispense_pre(dimension, pos, target, item, cancel);
    }
}
