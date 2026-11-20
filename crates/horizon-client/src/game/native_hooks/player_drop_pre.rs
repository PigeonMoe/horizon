use super::*;
use crate::game::Dim;

#[derive(PartialEq)]
struct GuestDropState {
    uuid: u128,
    entity: i32,
    window: Option<i32>,
    menu_generation: u64,
    game_mode: i32,
}

#[derive(PartialEq)]
struct DropState {
    dimension: Dim,
    feet: [f64; 3],
    eye: [f64; 3],
    guest: Option<GuestDropState>,
}

impl Game<'_> {
    pub(in crate::game) fn native_player_drop_pre(
        &mut self,
        player: u128,
        dimension: &str,
        position: [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_player_drop_pre(player, dimension, position, cancel);
    }

    pub(in crate::game) fn player_drop_allowed(&mut self, conn: Option<u32>) -> bool {
        let Some(before) = self.manual_drop_state(conn) else {
            return false;
        };
        let player = before
            .guest
            .as_ref()
            .map_or_else(|| self.native_acting_id(), |guest| guest.uuid);
        let mut cancel = false;
        self.native_player_drop_pre(
            player,
            before.dimension.id(),
            Self::player_drop_origin(before.eye),
            &mut cancel,
        );
        // A mutable hook can teleport the player or replace their menu. An
        // allowed drop applies only to the actor and origin shown to the hook.
        !cancel && self.manual_drop_state(conn).as_ref() == Some(&before)
    }

    fn manual_drop_state(&self, conn: Option<u32>) -> Option<DropState> {
        let dimension = self.dim;
        let guest = match conn {
            Some(conn) => {
                let guest = self.lan.as_ref()?.hub.guest(conn)?;
                if guest.dimension != dimension.id()
                    || guest.disconnect_pending
                    || guest.spectator()
                {
                    return None;
                }
                Some(guest)
            }
            None => None,
        };
        let (feet, eye) = if let Some(guest) = guest.filter(|_| conn != self.lan_acting) {
            (guest.position, guest.eye())
        } else {
            if self.spectator() {
                return None;
            }
            let position = self.player.as_ref()?.position;
            ([position.x, position.y, position.z], self.eye_position()?)
        };
        Some(DropState {
            dimension,
            feet,
            eye,
            guest: guest.map(|guest| GuestDropState {
                uuid: guest.uuid,
                entity: guest.entity_id,
                window: guest.menu.as_ref().map(|menu| menu.window),
                menu_generation: guest.menu_generation,
                game_mode: guest.game_mode,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::lan::{Out, PlayerSender, menu::input};
    use horizon_protocol::packets::play::{self, Clientbound as C, Serverbound as S};
    use mod_api::{Flow, Hooks, ModEntry, PlayerDropPreEvent, ServerHost};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };

    type DropPosition = (u128, String, [f64; 3]);

    struct DropPolicy {
        seen: Arc<Mutex<Vec<DropPosition>>>,
        allow: Arc<AtomicBool>,
    }

    #[mod_api::__subscriptions]
    impl Hooks for DropPolicy {
        fn player_drop_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut PlayerDropPreEvent<'_>,
        ) -> Flow {
            self.seen
                .lock()
                .unwrap()
                .push((event.player, event.dimension.into(), event.position));
            if self.allow.load(Ordering::Relaxed) {
                Flow::Continue
            } else {
                Flow::Cancel
            }
        }
    }

    fn click(container: i32, slot: i16, input: i32) -> S {
        S::ContainerClick(play::ContainerClick {
            container,
            state: 0,
            slot,
            button: 0,
            input,
            changed: Vec::new(),
            carried: None,
        })
    }

    fn menu_resynced(rx: &mpsc::Receiver<Out>, window: i32) -> bool {
        rx.try_iter().flat_map(|out| match out {
            Out::Packet(bytes) => vec![bytes],
            Out::Packets(packets) => packets,
            Out::Disconnect(_) => panic!("cancelled drop disconnected guest"),
        }).any(|body| {
            let mut read = horizon_protocol::buf::Reader::new(&body);
            let id = read.varint().unwrap();
            matches!(C::decode(id, read.rest()).unwrap(), C::ContainerSetContent(p) if p.container == window)
        })
    }

    #[test]
    #[ignore = "daily"]
    fn cancelled_manual_drops_preserve_the_stack_and_resync_the_menu() {
        crate::entry::install_client_hooks();
        let seen = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "deny_drops",
                Box::new(DropPolicy {
                    seen: seen.clone(),
                    allow: Arc::default(),
                }),
            )]
            .into(),
            || {
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.build_registries();
                    crate::game::native_player_death::tests::add_guest(game);
                    let (tx, rx) = mpsc::channel();
                    let stack = Some(ItemStack::new("minecraft:stone", 3));
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                        guest.position = [4.5, 80.0, 4.5];
                        guest.loaded = true;
                        guest.inventory[0] = stack.clone();
                    });
                    let guest = |game: &Game<'_>| {
                        game.lan.as_ref().unwrap().hub.guests[0].inventory[0].clone()
                    };
                    let [x, y, z] = game.lan.as_ref().unwrap().hub.guests[0].eye();
                    let expected = (123, game.dim.id().to_owned(), [x, y - f64::from(0.3f32), z]);

                    game.lan_guest_packet(
                        1,
                        S::PlayerAction(play::PlayerAction {
                            action: 5,
                            pos: [0; 3],
                            direction: 0,
                            sequence: 0,
                        }),
                    );
                    assert!(menu_resynced(&rx, 0));
                    assert_eq!(guest(game), stack);

                    game.lan_guest_packet(1, click(0, 36, input::THROW));
                    assert!(menu_resynced(&rx, 0));
                    assert_eq!(guest(game), stack);

                    let block = (4, 80, 4);
                    let chest = vec![stack.clone(); 27];
                    game.world
                        .block_entities
                        .insert(block, BlockEntity::Chest(chest.clone()));
                    let window = game
                        .lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .open_menu_window(
                            1,
                            "minecraft:generic_9x3",
                            Tag::String("Chest".into()),
                            Some(block),
                            Slots::Block(block),
                            chest.clone(),
                            None,
                        )
                        .unwrap();
                    for _ in rx.try_iter() {}

                    game.lan_guest_packet(1, click(window, 0, input::THROW));
                    assert!(menu_resynced(&rx, window));
                    assert!(
                        matches!(game.world.block_entities.get(&block), Some(BlockEntity::Chest(slots)) if slots == &chest)
                    );

                    game.lan
                        .as_mut()
                        .unwrap()
                        .hub
                        .guests
                        .update(0, |guest| guest.carried = stack.clone());
                    game.lan_guest_packet(1, click(window, -999, input::PICKUP));
                    assert!(menu_resynced(&rx, window));
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, stack);
                    assert!(game.dropped.is_empty());
                    assert_eq!(*seen.lock().unwrap(), vec![expected; 4]);
                })
            },
        );
    }

    #[test]
    #[ignore = "daily"]
    fn specialized_windows_check_drop_permission_before_a_slot_or_cursor_changes() {
        use horizon_core::position::ChunkPos;

        crate::entry::install_client_hooks();
        let allow = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "drop_policy",
            Box::new(DropPolicy {
                seen: seen.clone(),
                allow: allow.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.build_registries();
                game.set_allow_commands(true);
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                game.block_ticks
                    .add_empty_chunk(ChunkPos::new(0, 0))
                    .unwrap();
                assert!(game.run_command("fill 0 199 0 15 199 15 stone"));
                crate::game::native_player_death::tests::add_guest(game);
                let (tx, rx) = mpsc::channel();
                game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                    guest.sender = PlayerSender::new(tx, Arc::default());
                    guest.position = [8.5, 200.0, 8.5];
                    guest.loaded = true;
                });
                let stack = Some(ItemStack::new("minecraft:stone", 3));
                let expected = (
                    123,
                    game.dim.id().to_owned(),
                    Game::player_drop_origin(game.lan.as_ref().unwrap().hub.guests[0].eye()),
                );
                for (index, block, slot) in [
                    (0, "crafting_table", 37),
                    (1, "furnace", 30),
                    (2, "anvil", 30),
                ] {
                    allow.store(false, Ordering::Relaxed);
                    let drops_before = game.dropped.len();
                    let z = 7 + index;
                    assert!(game.run_command(&format!("setblock 10 200 {z} {block}")));
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory[0] = stack.clone();
                    });
                    game.lan_guest_packet(
                        1,
                        S::UseItemOn(play::UseItemOn {
                            hand: 0,
                            pos: [10, 200, z],
                            face: 1,
                            cursor: [0.5, 1.0, 0.5],
                            inside: false,
                            world_border: false,
                            sequence: index + 1,
                        }),
                    );
                    let window = rx
                        .try_iter()
                        .flat_map(|out| match out {
                            Out::Packet(bytes) => vec![bytes],
                            Out::Packets(packets) => packets,
                            Out::Disconnect(_) => panic!("guest disconnected"),
                        })
                        .find_map(|body| {
                            let mut read = horizon_protocol::buf::Reader::new(&body);
                            let id = read.varint().unwrap();
                            match C::decode(id, read.rest()).unwrap() {
                                C::OpenScreen(screen) => Some(screen.container),
                                _ => None,
                            }
                        })
                        .unwrap_or_else(|| panic!("{block} did not open"));
                    for _ in rx.try_iter() {}

                    game.lan_guest_packet(1, click(window, slot, input::THROW));
                    assert!(menu_resynced(&rx, window), "{block} throw");
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].inventory[0], stack);
                    assert_eq!(game.dropped.len(), drops_before);

                    game.lan_guest_packet(1, click(window, slot, input::PICKUP));
                    for _ in rx.try_iter() {}
                    game.lan_guest_packet(1, click(window, -999, input::PICKUP));
                    assert!(menu_resynced(&rx, window), "{block} cursor");
                    assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].carried, stack);
                    assert_eq!(game.dropped.len(), drops_before);

                    allow.store(true, Ordering::Relaxed);
                    game.lan_guest_packet(1, click(window, -999, input::PICKUP));
                    assert!(game.lan.as_ref().unwrap().hub.guests[0].carried.is_none());
                    assert_eq!(game.dropped.len(), drops_before + 1, "{block} allowed drop");
                    assert_eq!(game.dropped[drops_before].stack.count, 3);
                    let position = game.dropped[drops_before].position;
                    assert_eq!([position.x, position.y, position.z], expected.2);

                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.inventory[0] = stack.clone();
                    });
                    game.lan_guest_packet(1, click(window, slot, input::THROW));
                    assert_eq!(
                        game.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                            .as_ref()
                            .unwrap()
                            .count,
                        2
                    );
                    assert_eq!(
                        game.dropped.len(),
                        drops_before + 2,
                        "{block} allowed slot drop"
                    );
                    assert_eq!(game.dropped[drops_before + 1].stack.count, 1);
                    game.lan_guest_packet(
                        1,
                        S::ContainerClose(play::ContainerClose { container: window }),
                    );
                    for _ in rx.try_iter() {}
                }
                assert_eq!(*seen.lock().unwrap(), vec![expected; 12]);
            });
        });
    }
}
