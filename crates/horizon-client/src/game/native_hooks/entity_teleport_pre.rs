use super::*;
use horizon_core::geometry::Vec3;

pub(in crate::game) struct VehicleRiderRecovery {
    player: u128,
    pos: [f64; 3],
    rotation: [f32; 2],
}

impl Game<'_> {
    pub(in crate::game) fn native_entity_teleport_pre(
        &self,
        call: mod_api::__private::EntityTeleportPreCall<'_>,
    ) {
        self.fire_entity_teleport_pre(call);
    }

    /// The hull is still held out. Its movement supplies both poses to listeners.
    pub(in crate::game) fn vehicle_recovery_if_rollback_denied(
        &self,
        entity: u32,
        player: u128,
        (from, from_rotation): (Vec3, [f32; 2]),
        (to, to_rotation): (Vec3, [f32; 2]),
    ) -> Option<VehicleRiderRecovery> {
        if !mod_api::__private::listened(mod_api::HookId::EntityTeleportPre) {
            return None;
        }
        let mut cancel = false;
        self.native_entity_teleport_pre(mod_api::__private::EntityTeleportPreCall {
            entity,
            dimension: self.dim.id(),
            from: from.components(),
            to: to.components(),
            from_rotation,
            to_rotation,
            cause: "vehicle_rollback",
            cancel: &mut cancel,
        });
        cancel.then_some(VehicleRiderRecovery {
            player,
            pos: to.components(),
            rotation: to_rotation,
        })
    }

    /// Complete a failed hull teleport once its new position is public.
    pub(in crate::game) fn complete_vehicle_rider_recovery(
        &mut self,
        id: u32,
        recovery: Option<VehicleRiderRecovery>,
    ) -> bool {
        let Some(recovery) = recovery else {
            return false;
        };
        let Some(vehicle) = self.entity(id) else {
            return false;
        };
        self.entity_eject_passengers(vehicle);
        if let Some(player) = self.player(recovery.player) {
            let _ = self.player_teleport_checked(
                player,
                self.dim.id(),
                recovery.pos,
                Some(recovery.rotation),
            );
        }
        true
    }
}
