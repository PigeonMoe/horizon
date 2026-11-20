use super::*;
use crate::game::{
    native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
};
use horizon_client::{
    chunk::Chunk,
    lan::{Out, PlayerSender},
    mobs::Mob,
};
use horizon_core::position::ChunkPos;
use horizon_protocol::{
    buf::Reader,
    packets::play::{Clientbound, MoveVehicleRequest},
};
use mod_api::{
    EntityTeleportPreEvent, Flow, Hooks, Host, ModEntry, PlayerTeleportPreEvent,
    RiddenVehicleMovePreEvent, ServerHost,
};
use std::sync::{Mutex, mpsc};

const BOAT: u32 = 900;
const CAMEL: u32 = 901;
const FIRST: u128 = 13;
const SECOND: u128 = 12;
const START: Vec3 = Vec3::new(7.875, 200.0, 8.5);

#[derive(Clone, Copy, Debug, PartialEq)]
enum Policy {
    Allow,
    Stop,
    RecoverHull,
    RecoverRider,
    DenyRiderReturn,
}

#[derive(Debug, PartialEq)]
enum Attempt {
    Move(u128),
    HullTeleport,
    RiderTeleport(u128),
}

struct Boundary {
    policy: Policy,
    attempts: Arc<Mutex<Vec<Attempt>>>,
}

#[mod_api::__subscriptions]
impl Hooks for Boundary {
    fn ridden_vehicle_move_pre(
        &self,
        _: &dyn Host,
        event: &mut RiddenVehicleMovePreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.vehicle, BOAT);
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.from, START.components());
        assert!(event.to[0] >= 8.0);
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::Move(event.player));
        match self.policy {
            Policy::Allow => Flow::Continue,
            Policy::Stop => Flow::Cancel,
            _ => {
                event.paper_rollback = true;
                Flow::Cancel
            }
        }
    }

    fn entity_teleport_pre(&self, _: &dyn Host, event: &mut EntityTeleportPreEvent<'_>) -> Flow {
        assert_eq!(event.entity, BOAT);
        assert_eq!(event.cause, "vehicle_rollback");
        assert_eq!(event.dimension, "minecraft:overworld");
        assert!(event.from[0] >= 8.0);
        assert_eq!(event.to, START.components());
        self.attempts.lock().unwrap().push(Attempt::HullTeleport);
        match self.policy {
            Policy::RecoverHull => Flow::Continue,
            Policy::RecoverRider | Policy::DenyRiderReturn => Flow::Cancel,
            _ => panic!("the policy didn't request a hull teleport"),
        }
    }

    fn player_teleport_pre(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerTeleportPreEvent<'_>,
    ) -> Flow {
        assert_eq!(event.to, START.components());
        self.attempts
            .lock()
            .unwrap()
            .push(Attempt::RiderTeleport(event.player));
        match self.policy {
            Policy::RecoverRider => Flow::Continue,
            Policy::DenyRiderReturn => Flow::Cancel,
            _ => panic!("the hull did not need to eject a rider"),
        }
    }
}

fn launch(game: &mut Game<'_>) {
    game.build_registries();
    game.set_allow_commands(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    game.block_ticks
        .add_empty_chunk(ChunkPos::new(0, 0))
        .unwrap();
    assert!(game.run_command("forceload add 0 0"));
    let mut boat = Boat::new(BOAT, "oak".into(), START, 20.0);
    boat.no_gravity = true;
    boat.velocity.x = 0.5;
    game.boats.push(boat);
}

#[test]
#[ignore = "daily"]
fn a_server_driven_boat_checks_the_hull_return_and_recovers_only_the_denied_rider() {
    run_on_guest_stack(|| {
        for policy in [
            Policy::Allow,
            Policy::Stop,
            Policy::RecoverHull,
            Policy::RecoverRider,
            Policy::DenyRiderReturn,
        ] {
            let attempts = Arc::new(Mutex::new(Vec::new()));
            let mods = [ModEntry::new(
                "boat-rollback",
                Box::new(Boundary {
                    policy,
                    attempts: attempts.clone(),
                }),
            )];
            mod_platform::testing::with_mods(mods.into(), || {
                with_guests(|game| {
                    launch(game);
                    let kind = game.res.mob_types().get("minecraft:camel").unwrap().clone();
                    game.mobs.push(Mob::new(CAMEL, kind, START, 0.0));
                    let boat = game.entity(BOAT).unwrap();
                    let camel = game.entity(CAMEL).unwrap();
                    let first = game.player(FIRST).unwrap().entity();
                    let second = game.player(SECOND).unwrap().entity();
                    assert!(game.entity_start_riding(camel, boat, true));
                    assert!(game.entity_start_riding(first, camel, true));
                    assert!(game.entity_start_riding(second, camel, true));
                    assert_eq!(
                        game.entity_indirect_passengers(boat),
                        Some(vec![camel, first, second])
                    );

                    // Its direct passenger is a mob, so the server moves the hull.
                    game.tick_boats();
                    let mut expected = vec![Attempt::Move(FIRST)];
                    let hull = &game.boats[0];
                    match policy {
                        Policy::Allow => {
                            expected.push(Attempt::Move(SECOND));
                            assert!(hull.position.x >= 8.0);
                            assert_eq!(game.entity_vehicle(camel), Some(boat));
                            assert_eq!(game.entity_vehicle(first), Some(camel));
                        }
                        Policy::Stop | Policy::RecoverHull => {
                            if policy == Policy::RecoverHull {
                                expected.push(Attempt::HullTeleport);
                            }
                            assert_eq!(hull.position, START);
                            assert_eq!(hull.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(camel), Some(boat));
                            assert_eq!(game.entity_vehicle(first), Some(camel));
                        }
                        Policy::RecoverRider | Policy::DenyRiderReturn => {
                            expected.extend([Attempt::HullTeleport, Attempt::RiderTeleport(FIRST)]);
                            assert!(hull.position.x >= 8.0);
                            assert_eq!(hull.velocity, Vec3::ZERO);
                            assert_eq!(game.entity_vehicle(camel), None);
                            if policy == Policy::RecoverRider {
                                assert_eq!(game.entity_vehicle(first), None);
                                assert_eq!(game.entity_position(first), Some(START.components()));
                            } else {
                                assert_eq!(game.entity_vehicle(first), Some(camel));
                            }
                        }
                    }
                    assert_eq!(game.entity_vehicle(second), Some(camel));
                    assert_eq!(*attempts.lock().unwrap(), expected, "{policy:?}");
                });
            });
        }
    });
}

#[test]
#[ignore = "daily"]
fn a_rejected_guest_boat_packet_keeps_its_immediate_correction() {
    run_on_guest_stack(|| {
        let attempts = Arc::new(Mutex::new(Vec::new()));
        let mods = [ModEntry::new(
            "boat-packet",
            Box::new(Boundary {
                policy: Policy::RecoverRider,
                attempts: attempts.clone(),
            }),
        )];
        mod_platform::testing::with_mods(mods.into(), || {
            with_guests(|game| {
                launch(game);
                let rider = game.player(FIRST).unwrap().entity();
                let boat = game.entity(BOAT).unwrap();
                assert!(game.entity_start_riding(rider, boat, true));
                let (tx, packets) = mpsc::channel();
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update_connection(FIRST as u32, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                        guest.loaded = true;
                        guest.load_timeout = 0;
                        guest.awaiting_teleport = None;
                    })
                    .unwrap();

                game.guest_move_vehicle(
                    FIRST as u32,
                    &MoveVehicleRequest {
                        position: [START.x + 0.5, START.y, START.z],
                        yaw: 20.0,
                        pitch: 0.0,
                        on_ground: false,
                    },
                );
                assert_eq!(game.boats[0].position, START);
                assert_eq!(game.boats[0].velocity, Vec3::ZERO);
                assert_eq!(game.entity_vehicle(rider), Some(boat));
                assert_eq!(*attempts.lock().unwrap(), [Attempt::Move(FIRST)]);
                let corrections: Vec<_> = packets
                    .try_iter()
                    .filter_map(|out| {
                        let Out::Packet(body) = out else {
                            return None;
                        };
                        let mut reader = Reader::new(&body);
                        let id = reader.varint().unwrap();
                        if let Ok(Clientbound::MoveVehicle(packet)) =
                            Clientbound::decode(id, reader.rest())
                        {
                            Some((packet.position, packet.yaw))
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(corrections, [(START.components(), 20.0)]);

                game.tick_boats();
                assert_eq!(game.boats[0].position, START);
                assert_eq!(game.entity_vehicle(rider), Some(boat));
                assert_eq!(*attempts.lock().unwrap(), [Attempt::Move(FIRST)]);
            });
        });
    });
}
