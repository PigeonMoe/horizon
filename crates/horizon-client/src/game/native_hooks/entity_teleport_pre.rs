use super::*;
use crate::game::{commands::Target, native_entities::target_id, native_entity_moves::in_bounds};
use horizon_core::geometry::Vec3;

pub(in crate::game) enum VehicleRollback {
    Redirect {
        pos: [f64; 3],
        rotation: [f32; 2],
    },
    Eject {
        player: u128,
        pos: [f64; 3],
        rotation: [f32; 2],
    },
}

impl Game<'_> {
    pub(in crate::game) fn native_entity_teleport_pre(
        &self,
        call: mod_api::__private::EntityTeleportPreCall<'_>,
    ) {
        self.fire_entity_teleport_pre(call);
    }

    /// The hull is still held out. Its movement supplies both poses to listeners.
    pub(in crate::game) fn edit_vehicle_rollback(
        &self,
        entity: u32,
        player: u128,
        (from, from_rotation): (Vec3, [f32; 2]),
        (original, original_rotation): (Vec3, [f32; 2]),
    ) -> Option<VehicleRollback> {
        if !mod_api::__private::listened(mod_api::HookId::EntityTeleportPre) {
            return None;
        }
        let mut cancel = false;
        let original = original.components();
        let mut to = original;
        let mut to_rotation = original_rotation;
        self.native_entity_teleport_pre(mod_api::__private::EntityTeleportPreCall {
            entity,
            dimension: self.dim.id(),
            from: from.components(),
            to: &mut to,
            from_rotation,
            to_rotation: &mut to_rotation,
            cause: "vehicle_rollback",
            cancel: &mut cancel,
        });
        // The normal rewind keeps its existing placement rules when a mod only observes.
        let changed = to.map(f64::to_bits) != original.map(f64::to_bits)
            || to_rotation.map(f32::to_bits) != original_rotation.map(f32::to_bits);
        if cancel
            || (changed && (!in_bounds(to) || to_rotation.iter().any(|angle| !angle.is_finite())))
        {
            return Some(VehicleRollback::Eject {
                player,
                pos: original,
                rotation: original_rotation,
            });
        }
        changed.then_some(VehicleRollback::Redirect {
            pos: to,
            rotation: to_rotation,
        })
    }

    /// Complete an edited or refused hull teleport once the vehicle is available.
    pub(in crate::game) fn finish_vehicle_rollback(
        &mut self,
        target: Target,
        rollback: Option<VehicleRollback>,
    ) -> bool {
        let Some(rollback) = rollback else {
            return false;
        };
        match rollback {
            VehicleRollback::Redirect { pos, rotation } => {
                self.place_vehicle_rollback(target, pos, rotation)
            }
            VehicleRollback::Eject {
                player,
                pos,
                rotation,
            } => {
                let Some(vehicle) = target_id(target).and_then(|id| self.entity(id)) else {
                    return false;
                };
                self.entity_eject_passengers(vehicle);
                if let Some(player) = self.player(player) {
                    let _ =
                        self.player_teleport_checked(player, self.dim.id(), pos, Some(rotation));
                }
                true
            }
        }
    }

    /// Place the vehicle together with the riders it carries.
    pub(in crate::game) fn place_vehicle_rollback(
        &mut self,
        target: Target,
        pos: [f64; 3],
        rotation: [f32; 2],
    ) -> bool {
        self.place_teleport_filed(target, pos, Some(rotation), [false; 3], true)
            .is_some()
    }
}
