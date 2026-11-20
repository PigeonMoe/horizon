use super::initialize;
use horizon_client::lan::guest::InboxItem;
use horizon_protocol::packets::play::{self, Serverbound};
use mod_api::{Flow, Hooks, ModEntry, PlayerSprintEvent, PlayerSprintPreEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Turn {
    Attempt { requested: bool, current: bool },
    Changed(bool),
}

struct Probe {
    observed: Arc<Mutex<Vec<Turn>>>,
    cancel_first_stop: AtomicBool,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn player_sprint_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerSprintPreEvent,
    ) -> Flow {
        assert_eq!(event.player.uuid(), 12);
        assert_eq!(host.players().acting_player(), Some(event.player));
        let current = event.player.entity().is_sprinting(host).unwrap();
        self.observed.lock().unwrap().push(Turn::Attempt {
            requested: event.sprinting,
            current,
        });
        if !event.sprinting && self.cancel_first_stop.swap(false, Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }

    fn player_sprint(&self, host: &mut dyn ServerHost, event: &mut PlayerSprintEvent<'_>) {
        assert_eq!(event.player, 12);
        let player = host.players().acting_player().unwrap();
        assert_eq!(player.entity().is_sprinting(host), Some(event.sprinting));
        self.observed
            .lock()
            .unwrap()
            .push(Turn::Changed(event.sprinting));
    }
}

#[test]
#[ignore = "daily"]
fn repeated_sprint_commands_are_admitted_in_order_before_state_changes() {
    crate::game::native_entity_refs::tests::run_on_guest_stack(|| {
        let observed = Arc::new(Mutex::new(Vec::new()));
        let mods = vec![ModEntry::new(
            "sprint-attempt",
            Box::new(Probe {
                observed: Arc::clone(&observed),
                cancel_first_stop: AtomicBool::new(true),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::entry::install_client_hooks();
            crate::game::native_guest_turn_tests::with_guests(|game| {
                let host = game.lan.as_mut().unwrap();
                host.hub
                    .update_guest(12, |guest| {
                        assert!(initialize(game.motion, guest));
                        guest.loaded = true;
                        guest.load_timeout = 0;
                    })
                    .unwrap();
                let inbox = host.hub.guests.inbox_mut(0).unwrap();
                for action in [1, 1, 2, 2, 2] {
                    inbox.push_back(InboxItem::Packet(
                        Serverbound::PlayerCommand(play::PlayerCommand {
                            id: 12,
                            action,
                            data: 0,
                        }),
                        None,
                    ));
                }
                game.tick_lan_packets();
                assert_eq!(
                    *observed.lock().unwrap(),
                    [
                        Turn::Attempt {
                            requested: true,
                            current: false
                        },
                        Turn::Changed(true),
                        Turn::Attempt {
                            requested: true,
                            current: true
                        },
                        Turn::Attempt {
                            requested: false,
                            current: true
                        },
                        Turn::Attempt {
                            requested: false,
                            current: true
                        },
                        Turn::Changed(false),
                        Turn::Attempt {
                            requested: false,
                            current: false
                        },
                    ]
                );
                assert!(!game.lan.as_ref().unwrap().hub.guest(12).unwrap().sprinting);
            });
        });
    });
}
