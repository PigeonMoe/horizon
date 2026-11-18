use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_pickup_arrow_pre(
        &mut self,
        player: u128,
        dimension: &str,
        position: [f64; 3],
        item: &str,
        cancel: &mut bool,
    ) {
        self.fire_player_pickup_arrow_pre(player, dimension, position, item, cancel);
    }
}
