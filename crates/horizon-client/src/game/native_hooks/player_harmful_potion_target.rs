use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_harmful_potion_target(
        &self,
        player: u128,
        dimension: &str,
        target: u32,
        target_kind: &str,
        position: [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_harmful_potion_target(
            player,
            dimension,
            target,
            target_kind,
            position,
            cancel,
        );
    }
}
