use super::*;
use crate::game::{api_values::TryIntoApi, spill::BlockDrops};

impl Game<'_> {
    pub(in crate::game) fn observe_player_block_drops(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        drops: &BlockDrops,
    ) {
        if drops.canceled
            || !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::PlayerBlockDropsPost)
        {
            return;
        }
        if let (Some(player), Ok(items)) = (
            self.native_acting_player(),
            drops.items.try_into_api(),
        ) {
            self.native_player_block_drops_post(super::fire::PlayerBlockDropsPostCall {
                player,
                pos,
                state,
                drops: &items,
            });
        }
    }

    pub(in crate::game) fn native_player_block_drops_post(
        &mut self,
        call: super::fire::PlayerBlockDropsPostCall<'_>,
    ) {
        self.fire_player_block_drops_post(call);
    }
}
