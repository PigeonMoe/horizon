use super::*;
use crate::game::commands::Target;

impl Game<'_> {
    pub(in crate::game) fn native_vehicle_damage_pre(
        &self,
        vehicle: u32,
        kind: &str,
        dimension: &str,
        position: [f64; 3],
        player: Option<&mod_api::DamagePlayer>,
        cancel: &mut bool,
    ) {
        self.fire_vehicle_damage_pre(vehicle, kind, dimension, position, player, cancel);
    }

    pub(in crate::game) fn vehicle_damage_allowed(
        &self,
        id: u32,
        kind: impl FnOnce() -> std::borrow::Cow<'static, str>,
        position: [f64; 3],
        cause: Option<Target>,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::VehicleDamagePre)
        {
            return true;
        }
        let player = cause.and_then(|source| self.damage_player_from_target(source));
        let mut cancel = false;
        self.native_vehicle_damage_pre(
            id,
            &kind(),
            self.dim.id(),
            position,
            player.as_ref(),
            &mut cancel,
        );
        !cancel
    }
}
