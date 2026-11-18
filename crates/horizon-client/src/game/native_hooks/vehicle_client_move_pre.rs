use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_vehicle_client_move_pre(
        &self,
        player: u128,
        vehicle: u32,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_vehicle_client_move_pre(player, vehicle, dimension, from, to, cancel);
    }
}
