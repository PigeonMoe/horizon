use super::*;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    pub(in crate::game) fn native_entity_item_place_pre(
        &self,
        call: mod_api::__private::EntityItemPlacePreCall<'_>,
    ) {
        self.fire_entity_item_place_pre(call);
    }

    pub(in crate::game) fn entity_item_place_cancelled(
        &mut self,
        kind: &str,
        position: Vec3,
        vehicle_stack: Option<&ItemStack>,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::EntityItemPlacePre)
        {
            return false;
        }
        // Boats and carts apply their stack data after insertion; inspect its
        // position with the same loader before letting the item place them.
        let position = vehicle_stack
            .and_then(|stack| {
                horizon_client::block_entity_item::spawn_egg_entity_data(
                    stack,
                    kind,
                    self.acting_player_is_op(),
                )
            })
            .and_then(horizon_client::entity_nbt::boat_from_nbt)
            .map_or(position, |(at, _, _)| at);
        let player = horizon_client::lan::playerdata::ints_uuid(self.acting_player_uuid());
        let mut cancel = false;
        self.native_entity_item_place_pre(mod_api::__private::EntityItemPlacePreCall {
            dimension: self.dim.id(),
            kind,
            position: position.components(),
            player,
            cancel: &mut cancel,
        });
        if cancel {
            self.use_failed = true;
        }
        cancel
    }
}

#[cfg(test)]
mod tests {
    use crate::game::Hit;
    use crate::game::native_entity_refs::tests::{bind_local_player, with_game};
    use horizon_client::{chunk::Chunk, motion::PlayerMotion};
    use horizon_core::nbt::Tag;
    use mod_api::{EntityItemPlacePreEvent, Flow, Hooks, ModEntry, Mods};
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct State {
        cancel: bool,
        placements: Vec<(String, [f64; 3])>,
    }

    struct Guard(Arc<Mutex<State>>);

    #[mod_api::__subscriptions]
    impl Hooks for Guard {
        fn entity_item_place_pre(
            &self,
            _: &dyn mod_api::Host,
            event: &mut EntityItemPlacePreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(event.player, 17);
            let mut state = self.0.lock().unwrap();
            state
                .placements
                .push((event.kind.to_owned(), event.position));
            if state.cancel {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn item_placement_veto_uses_the_final_entity_position_and_keeps_the_stack() {
        let state = Arc::new(Mutex::new(State::default()));
        let mods: Mods = vec![ModEntry::new(
            "placement-guard",
            Box::new(Guard(state.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(5.5, 67.0, 5.5)).unwrap());
                bind_local_player(game, 7, 17);
                game.pitch = 90.0;
                game.creative = false;
                let registry = game.pack.assets.registry;
                for (pos, block) in [
                    ([5, 64, 5], "minecraft:water"),
                    ([8, 64, 8], "minecraft:rail"),
                    ([13, 64, 8], "minecraft:stone"),
                    ([13, 64, 13], "minecraft:obsidian"),
                ] {
                    let block = registry.default_state(block).unwrap();
                    game.world.set(pos[0], pos[1], pos[2], block).unwrap();
                }
                let hit = |pos: [i32; 3], face| Hit {
                    pos,
                    face,
                    point: pos.map(|v| f64::from(v) + 0.5),
                    inside: false,
                };
                let placements = |game: &Game<'_>| {
                    game.boats.len()
                        + game.carts.len()
                        + game.armor_stands.len()
                        + game.crystals.len()
                };
                let boat = ItemStack::new("minecraft:oak_boat", 1);
                let boat_pos = game.boat_to_place(&boat).unwrap().position.components();
                let mut cart = ItemStack::new("minecraft:minecart", 1);
                let cart_pos = [8.5, 65.0, 13.5];
                cart.data_mut().set_other(
                    "minecraft:entity_data",
                    Some(Tag::compound([
                        ("id", Tag::String("minecraft:minecart".into())),
                        (
                            "Pos",
                            Tag::List(cart_pos.map(|v: f64| Tag::Double(v.to_bits())).to_vec()),
                        ),
                    ])),
                );
                for (stack, expected) in [
                    (boat, boat_pos),
                    (cart, cart_pos),
                    (
                        ItemStack::new("minecraft:armor_stand", 1),
                        [13.5, 65.0, 8.5],
                    ),
                    (
                        ItemStack::new("minecraft:end_crystal", 1),
                        [13.5, 65.0, 13.5],
                    ),
                ] {
                    let kind = stack.item.as_ref();
                    for cancel in [true, false] {
                        state.lock().unwrap().cancel = cancel;
                        game.use_failed = false;
                        let id = game.next_entity;
                        let inserted = placements(game);
                        *game.held_mut() = Some(stack.clone());
                        let handled = match kind {
                            "minecraft:oak_boat" => game.place_boat(&stack, None) == Some(true),
                            "minecraft:minecart" => {
                                game.place_minecart(&hit([8, 64, 8], [0, 1, 0]), &stack)
                            }
                            "minecraft:armor_stand" => {
                                game.place_armor_stand(&hit([13, 64, 8], [0, 1, 0]), &stack)
                            }
                            _ => game.place_end_crystal(&hit([13, 64, 13], [1, 0, 0]), &stack),
                        };
                        assert!(handled, "{kind}");
                        assert_eq!(game.use_failed, cancel, "{kind}");
                        assert_eq!(game.next_entity, id + u32::from(!cancel), "{kind}");
                        assert_eq!(placements(game), inserted + usize::from(!cancel), "{kind}");
                        assert_eq!(game.held(), cancel.then_some(&stack), "{kind}");
                        assert_eq!(
                            std::mem::take(&mut state.lock().unwrap().placements),
                            [(kind.to_owned(), expected)],
                        );
                    }
                }
            });
        });
    }
}
