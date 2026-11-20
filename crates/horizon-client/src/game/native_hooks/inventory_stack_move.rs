use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_inventory_stack_move(
        &self,
        call: mod_api::__private::InventoryStackMoveCall<'_>,
    ) {
        self.fire_inventory_stack_move(call);
    }
}
