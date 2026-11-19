use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_enchant_item_pre(
        &mut self,
        call: mod_api::__private::PlayerEnchantItemPreCall<'_>,
    ) {
        self.fire_player_enchant_item_pre(call);
    }
}
