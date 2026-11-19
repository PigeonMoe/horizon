use super::*;
use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_item_places::tests::{guest, open_to_lan};
use horizon_client::{
    chunk::Chunk,
    lan::{Out, PlayerSender},
};
use mod_api::{Flow, Hooks, HostSide, InteractionResult, RightClickBlockEvent};
use std::sync::{Arc, mpsc};

struct Refusal;

#[mod_api::__subscriptions]
impl Hooks for Refusal {
    fn right_click_block(&self, host: HostSide<'_>, event: &mut RightClickBlockEvent<'_>) -> Flow {
        if matches!(host, HostSide::Server(_)) && event.click.pos == [4, 63, 5] {
            *event.cancellation_result = InteractionResult::Fail;
            return Flow::Cancel;
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn a_refused_guest_placement_resends_the_unchanged_hand() {
    crate::game::in_own_process(
        "game::lan_actions::refused_placement_inventory_tests::a_refused_guest_placement_resends_the_unchanged_hand",
        refused_placement_inventory_isolated,
    );
}

fn refused_placement_inventory_isolated() {
    type Stack = horizon_client::items::ItemStack;
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "refuse_placement",
        Box::new(Refusal),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.set(4, 63, 5, stone).unwrap();
            let (tx, packets) = mpsc::channel();
            let mut player = guest(7);
            player.position = [4.5, 64.0, 3.5];
            player.loaded = true;
            player.selected = 6;
            player.inventory[6] = Some(Stack::new("minecraft:oak_planks", 3));
            player.offhand = Some(Stack::new("minecraft:oak_planks", 5));
            player.sender = PlayerSender::new(tx, Arc::default());
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let conn = game.lan.as_ref().unwrap().hub.guests[0].conn;
            game.lan.as_mut().unwrap().hub.send_inventory(0);
            packets.try_iter().for_each(drop);

            for (hand, slot, count) in [(Hand::Main, 42, 3), (Hand::Off, 45, 5)] {
                let hit = Hit {
                    pos: [4, 63, 5],
                    face: [0, 1, 0],
                    point: [4.5, 64.0, 5.5],
                    inside: false,
                };
                game.guest_use_item_on(conn, 0, hit, hand).unwrap();
                assert!(game.world.is_air(game.world.block(4, 64, 5)));
                let inventory = &game.lan.as_ref().unwrap().hub.guests[0];
                assert_eq!(
                    inventory
                        .menu_slot(usize::try_from(slot).unwrap())
                        .unwrap()
                        .count,
                    count
                );
                let restored: Vec<_> = packets
                    .try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("unexpected disconnect"),
                    })
                    .filter_map(|body| {
                        let mut reader = horizon_protocol::buf::Reader::new(&body);
                        match C::decode(reader.varint().unwrap(), reader.rest()).unwrap() {
                            C::ContainerSetSlot(update) if update.container == 0 => {
                                update.item.map(|stack| (update.slot, stack.count))
                            }
                            _ => None,
                        }
                    })
                    .collect();
                assert_eq!(restored, [(slot, i32::try_from(count).unwrap())]);
            }
        });
    });
}
