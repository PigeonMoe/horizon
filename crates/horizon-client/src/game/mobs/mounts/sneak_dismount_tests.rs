//! A sneaking rider gets off its mount (`Player.rideTick`: `wantsToStopRiding`, which is
//! `isShiftKeyDown`), unless an `entity_mount` listener cancels the dismount (NeoForge's
//! `EntityMountEvent`). Kept on, the mount still reads that tick's input, sneak included
//! (`LivingEntity.getRiddenInput` reads the controller), on the local player's route and on a
//! server player's (arismoko/horizon#2739).
use super::camel_jump_tests::game;
use crate::game::Game;
use horizon_client::{mobs::equine, sprint::Keys};
use mod_api::{EntityMountEvent, Flow, Hooks, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU32, Ordering},
};

/// Cancels every dismount while `keep` is set, and counts the dismounts it hears.
struct KeepRider {
    keep: Arc<AtomicBool>,
    leaves: Arc<AtomicU32>,
}
#[mod_api::__subscriptions]
impl Hooks for KeepRider {
    fn entity_mount(&self, _: &mut dyn ServerHost, event: &mut EntityMountEvent<'_>) -> Flow {
        if event.mounting {
            return Flow::Continue;
        }
        self.leaves.fetch_add(1, Ordering::SeqCst);
        if self.keep.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Runs `test` on a server game with a horse (its id) and the `KeepRider` mod.
pub(in crate::game) fn with_horse(test: impl FnOnce(&mut Game<'_>, u32, &AtomicBool, &AtomicU32)) {
    let keep = Arc::new(AtomicBool::new(true));
    let leaves = Arc::new(AtomicU32::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "keep_rider",
        Box::new(KeepRider {
            keep: keep.clone(),
            leaves: leaves.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        let mut game = game();
        // A server hears its mounts' `entity_mount`.
        game.options.dedicated = Some(Vec::new());
        assert!(game.run_command("summon horse 8.5 100 8.5 {NoAI:1b,Silent:1b,Tame:1b}"));
        let horse = game
            .mobs
            .iter()
            .find(|m| &*m.kind.id == "minecraft:horse")
            .unwrap()
            .id;
        test(&mut game, horse, &keep, &leaves);
    });
}

fn rider(game: &Game<'_>, horse: u32) -> Option<equine::Rider> {
    game.mob_object(horse).unwrap().rider
}

#[test]
#[ignore = "daily"]
fn underwater_dismount_obeys_the_listener_without_sneak() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        assert!(game.run_command("fill 0 100 0 15 104 15 water"));
        assert!(game.rider_eyes_in_water(), "exercise the underwater branch");
        let keys = Keys {
            forward: true,
            ..Keys::default()
        };
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 1);
        assert_eq!(game.riding, Some(horse));
        let input = rider(game, horse).unwrap();
        assert_eq!(input.forward, 1.0);
        assert!(!input.sneak);
        keep.store(false, Ordering::SeqCst);
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(game.riding, None);
        assert!(rider(game, horse).is_none());
    });
}

#[test]
#[ignore = "daily"]
fn a_dead_mount_never_receives_input_even_when_dismount_is_cancelled() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        let i = game.mount_index().unwrap();
        game.mobs[i].health = 0.0;
        for cancelled in [true, false] {
            keep.store(cancelled, Ordering::SeqCst);
            assert!(game.tick_mount_riding(Keys {
                forward: true,
                jump: true,
                ..Keys::default()
            }));
            assert!(rider(game, horse).is_none(), "dead mounts receive no input");
            assert_eq!(game.riding, cancelled.then_some(horse));
        }
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
    });
}

#[test]
#[ignore = "daily"]
fn a_kept_local_rider_still_drives_and_sneaks() {
    with_horse(|game, horse, keep, leaves| {
        game.riding = Some(horse);
        let keys = Keys {
            forward: true,
            left: true,
            jump: true,
            shift: true,
            ..Keys::default()
        };
        assert!(game.tick_mount_riding(keys));
        assert_eq!(leaves.load(Ordering::SeqCst), 1, "the dismount was asked");
        assert_eq!(game.riding, Some(horse), "the listener kept the rider on");
        let r = rider(game, horse).expect("the kept mount reads the rider");
        assert_eq!((r.strafe, r.forward), (1.0, 1.0));
        assert!(r.jumping, "the jump key reaches the mount");
        assert!(r.sneak, "the mount sees sneak held");
        assert!(!r.client, "the local player's mount moves here");

        // Without the listener's veto, sneaking gets the rider off as before.
        keep.store(false, Ordering::SeqCst);
        assert!(game.tick_mount_riding(Keys {
            shift: true,
            ..Keys::default()
        }));
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(game.riding, None);
        assert!(rider(game, horse).is_none(), "the mount lost its rider");
    });
}

#[test]
#[ignore = "daily"]
fn a_kept_server_player_still_drives_and_sneaks() {
    // `PlayerInput` flags: forward 1, left 4, jump 16, shift 32.
    const INPUT: u8 = 1 | 4 | 16 | 32;
    const SHIFT: u8 = 32;
    with_horse(|game, horse, keep, leaves| {
        let set_input = |game: &mut Game<'_>, input: u8| {
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                (g.vehicle, g.vehicle_order, g.input) = (Some(horse), 1, input);
            });
        };
        let vehicle = |game: &Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].vehicle;
        set_input(game, INPUT);
        game.guest_vehicle_tick(2);
        assert_eq!(leaves.load(Ordering::SeqCst), 1, "the dismount was asked");
        assert_eq!(vehicle(game), Some(horse), "the listener kept the guest on");
        let r = rider(game, horse).expect("the kept mount reads the guest");
        assert_eq!((r.strafe, r.forward), (1.0, 1.0));
        assert!(r.jumping, "the jump key reaches the mount");
        assert!(r.sneak, "the mount sees sneak held");
        assert!(r.client, "the guest's own client moves its horse");

        // Without the listener's veto, sneaking gets the guest off as before.
        keep.store(false, Ordering::SeqCst);
        set_input(game, SHIFT);
        game.guest_vehicle_tick(2);
        assert_eq!(leaves.load(Ordering::SeqCst), 2);
        assert_eq!(vehicle(game), None);
        assert!(rider(game, horse).is_none(), "the mount lost its rider");
    });
}

#[test]
#[ignore = "daily"]
fn a_live_horse_checks_its_guests_sneak_dismount_at_their_chosen_side() {
    check_guest_horse_dismount(false);
}

#[test]
#[ignore = "daily"]
fn a_submerged_guests_horse_exit_checks_its_landing_without_sneak() {
    check_guest_horse_dismount(true);
}

fn check_guest_horse_dismount(submerged: bool) {
    use crate::game::{
        native_entity_refs::tests::run_on_guest_stack, native_guest_turn_tests::with_guests,
    };
    use horizon_client::pose::Pose;
    use horizon_core::{geometry::Vec3, position::ChunkPos};
    use mod_api::{PlayerPositionCommittedEvent, PlayerTeleportPreEvent};
    use std::sync::Mutex;

    const DENIED: u128 = 13;
    const ALLOWED: u128 = 12;

    #[derive(Debug, PartialEq)]
    struct Trace {
        player: u128,
        from: [f64; 3],
        to: [f64; 3],
        committed: bool,
    }

    struct Protect(Arc<Mutex<Vec<Trace>>>);
    #[mod_api::__subscriptions]
    impl Hooks for Protect {
        fn player_teleport_pre(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerTeleportPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.cause, "dismount");
            assert_eq!(event.from_level, "minecraft:overworld");
            assert_eq!(event.to_level, event.from_level);
            let player = host.player(event.player).unwrap().entity();
            assert_eq!(player.vehicle(host), None);
            assert_eq!(player.position(host), Some(event.from));
            self.0.lock().unwrap().push(Trace {
                player: event.player,
                from: event.from,
                to: event.to,
                committed: false,
            });
            if event.player == DENIED {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }

        fn player_position_committed(
            &self,
            host: &mut dyn ServerHost,
            event: &mut PlayerPositionCommittedEvent<'_>,
        ) {
            let player = host.player(event.player).unwrap().entity();
            assert_eq!(player.vehicle(host), None);
            assert_eq!(player.position(host), Some(event.to));
            self.0.lock().unwrap().push(Trace {
                player: event.player,
                from: event.from,
                to: event.to,
                committed: true,
            });
        }
    }

    run_on_guest_stack(move || {
        let traces = Arc::new(Mutex::new(Vec::new()));
        let keep = Arc::new(AtomicBool::new(submerged));
        let leaves = Arc::new(AtomicU32::new(0));
        mod_platform::testing::with_mods(
            vec![
                mod_api::ModEntry::new("horse-dismount-test", Box::new(Protect(traces.clone()))),
                mod_api::ModEntry::new(
                    "keep-rider",
                    Box::new(KeepRider {
                        keep: keep.clone(),
                        leaves: leaves.clone(),
                    }),
                ),
            ]
            .into(),
            || {
                with_guests(|game| {
                    game.options.dedicated = Some(Vec::new());
                    game.build_registries();
                    game.set_allow_commands(true);
                    game.world
                        .chunks
                        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                    game.block_ticks
                        .add_empty_chunk(ChunkPos::new(0, 0))
                        .unwrap();
                    assert!(game.run_command("forceload add 0 0"));
                    let registry = game.pack.assets.registry;
                    let stone = registry.default_state("minecraft:stone").unwrap();
                    let slab = crate::game::native_hooks::with_property(
                        registry,
                        registry.default_state("minecraft:stone_slab").unwrap(),
                        "type",
                        "bottom",
                    )
                    .unwrap();
                    for x in 6..=10 {
                        for z in 6..=10 {
                            game.world.set(x, 63, z, stone).unwrap();
                        }
                    }
                    // The right side fits only a crouching rider. The left
                    // side has a full standing space.
                    game.world.set(7, 64, 8, slab).unwrap();
                    game.world.set(7, 66, 8, stone).unwrap();
                    if submerged {
                        let water = registry.default_state("minecraft:water").unwrap();
                        for y in 64..=68 {
                            game.world.set(8, y, 8, water).unwrap();
                        }
                    }
                    game.settings.main_hand_left = false;
                    assert!(game.run_command("summon horse 8.5 64 8.5 {NoAI:1b,Tame:1b}"));
                    let horse = game
                        .mobs
                        .iter_mut()
                        .find(|mob| &*mob.kind.id == "minecraft:horse")
                        .unwrap();
                    horse.yaw = 0.0;
                    let horse_id = horse.id;
                    let horse = game.entity(horse_id).unwrap();
                    let motion = Vec3::new(0.2, -0.1, 0.0);
                    for (uuid, hand, x, y, pose) in [
                        (DENIED, 1, 7.0, 64.5, Pose::Crouching),
                        (ALLOWED, 0, 9.0, 64.0, Pose::Standing),
                    ] {
                        let conn = uuid as u32;
                        let player = game.player(uuid).unwrap().entity();
                        assert!(game.entity_start_riding(player, horse, true));
                        game.guest_boarded(conn, None);
                        game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                            guest.loaded = true;
                            guest.load_timeout = 0;
                            guest.awaiting_teleport = None;
                            guest.info.main_hand = hand;
                        });
                        game.lan_player_input(conn, if submerged { 0 } else { 32 }, true);
                        assert_eq!(
                            game.as_guest(conn, |game| game.rider_eyes_in_water()),
                            Some(submerged)
                        );
                        if submerged && uuid == DENIED {
                            game.guest_vehicle_tick(conn);
                            assert_eq!(leaves.load(Ordering::SeqCst), 1);
                            assert_eq!(game.command_guest(conn).unwrap().vehicle, Some(horse_id));
                            assert!(traces.lock().unwrap().is_empty());
                            keep.store(false, Ordering::SeqCst);
                        }
                        game.lan.as_mut().unwrap().hub.update_guest(conn, |guest| {
                            guest.motion.as_mut().unwrap().velocity = motion;
                        });
                        let from = game.command_guest(conn).unwrap().position;
                        game.guest_vehicle_tick(conn);

                        let records = std::mem::take(&mut *traces.lock().unwrap());
                        let allowed = uuid == ALLOWED;
                        assert_eq!(records.len(), if allowed { 2 } else { 1 });
                        let to = records[0].to;
                        assert_eq!([to[0].floor(), to[1], to[2].floor()], [x, y, 8.0]);
                        assert_eq!(
                            records[0],
                            Trace {
                                player: uuid,
                                from,
                                to,
                                committed: false
                            }
                        );
                        if allowed {
                            assert_eq!(
                                records[1],
                                Trace {
                                    player: uuid,
                                    from,
                                    to,
                                    committed: true
                                }
                            );
                        }
                        let guest = game.command_guest(conn).unwrap();
                        assert_eq!(guest.vehicle, None);
                        assert_eq!(guest.pose(), pose);
                        assert_eq!(guest.position, if allowed { to } else { from });
                        assert_eq!(guest.awaiting_teleport.unwrap().1, guest.position);
                        assert_eq!(guest.awaiting_keeps_motion, allowed);
                        assert_eq!(
                            guest.motion.as_ref().unwrap().velocity,
                            if allowed { motion } else { Vec3::ZERO }
                        );
                    }
                });
            },
        );
    });
}
