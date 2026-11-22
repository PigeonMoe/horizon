//! Permission for player abilities that target a location, such as lightning.
use super::Game;
use mod_api::{LevelRef, PlayerRef};

impl Game<'_> {
    pub(super) fn level_allow_player_attack_at(
        &self,
        level: LevelRef,
        player: PlayerRef,
        pos: [i32; 3],
    ) -> bool {
        if !self.level_in_place(level)
            || self.player_level(player) != Some(level)
            || !self.level_is_loaded(level, pos)
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::PVP)
        {
            return false;
        }
        if !mod_api::__private::listened(mod_api::HookId::PlayerAttackAtPre) {
            return true;
        }
        let mut cancel = false;
        self.native_player_attack_at_pre(mod_api::__private::PlayerAttackAtPreCall {
            player,
            dimension: self.dim.id(),
            pos,
            cancel: &mut cancel,
        });
        !cancel
    }
}
