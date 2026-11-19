//! The horse inventory for a server's players (`ServerPlayer.openHorseInventory`):
//! shift + use on a tamed horse, donkey, mule, camel, llama or nautilus, or the
//! inventory key while riding one (`ServerboundPlayerCommandPacket`
//! OPEN_INVENTORY), opens `HorseInventoryMenu` over the mount with
//! `ClientboundMountScreenOpenPacket`. Clicks run on the menu engine
//! (`Kind::Mount`) with the mount's saddle, body item and chest copied in and
//! written back, and the slots go to the client after each change, as
//! `broadcastChanges` does.
use super::{
    Game,
    lan_actions::guest_within_entity_range,
    lan_crafting::{DropMenu, GuestMenuClick},
};
use horizon_client::{
    items::ItemStack,
    menu::{Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open mount menu.
pub(super) struct GuestMount {
    pub(super) window: i32,
    pub(super) mob: u32,
    columns: usize,
    state: i32,
    /// The menu's slots and the carried stack the client was last sent.
    sent: Option<(Vec<Option<ItemStack>>, Option<ItemStack>)>,
}

impl Game<'_> {
    /// `ServerPlayer.openHorseInventory` for mount `mob`: the open menu
    /// closes, the client gets the mount screen and the menu's slots.
    pub(super) fn lan_open_mount(&mut self, conn: u32, mob: u32) {
        let Some(index) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let mount = &self.mobs[index];
        let columns = super::mount_menu::mount_columns(mount);
        let pos = mount.position.components().map(|axis| axis.floor() as i32);
        if !self.guest_may_open(conn, "minecraft:horse_inventory", Some(pos)) {
            return;
        }
        // ServerPlayer.openMenu closes the menu that was open.
        self.guest_close_all(conn);
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(window) = host.hub.open_mount_window(conn, columns as i32, mob as i32) else {
            return;
        };
        host.play.entry(conn).or_default().mount_window = Some(GuestMount {
            window,
            mob,
            columns,
            state: 0,
            sent: None,
        });
        self.lan_send_mount(conn);
        self.player_menu_opening(conn);
    }

    /// `HorseInventoryMenu` over mount `index`, with the player's inventory
    /// loaded by the caller.
    fn mount_engine_menu(&self, index: usize, columns: usize) -> Menu {
        let env = self.menu_env();
        let mut menu = Menu::new(Kind::Mount(columns));
        menu.set_registry_id(Some("horizon:mount_inventory"));
        menu.mount = Some(self.mobs[index].kind.id.to_string());
        let (saddle, body, items) = self.mount_contents(index);
        menu.load_slot(0, saddle);
        menu.load_slot(1, body.flatten());
        for (k, item) in items.into_iter().take(columns * 3).enumerate() {
            menu.set_slot(&env, 2 + k, item);
        }
        menu
    }

    /// The menu's slots (the mount's, then the inventory and the hotbar)
    /// and the carried stack, sent when they changed since the last time.
    fn lan_send_mount(&mut self, conn: u32) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(gm) = host.play.get(&conn).and_then(|p| p.mount_window.as_ref()) else {
            return;
        };
        let (mob, columns) = (gm.mob, gm.columns);
        let Some(index) = self.mobs.iter().position(|m| m.id == mob) else {
            return;
        };
        let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        let (saddle, body, items) = self.mount_contents(index);
        let mut slots = vec![saddle, body.flatten()];
        slots.extend(items.into_iter().take(columns * 3));
        slots.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let now = Some((slots, carried));
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(gm) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.mount_window.as_mut())
        else {
            return;
        };
        if gm.sent == now {
            return;
        }
        gm.state = (gm.state + 1) & 0x7FFF;
        let (window, state) = (gm.window, gm.state);
        let (slots, carried) = now.clone().unwrap_or_default();
        gm.sent = now;
        let content = C::ContainerSetContent(play::ContainerSetContent {
            container: window,
            state,
            items: slots.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        host.hub.send_to(conn, &content);
    }

    /// A click on the guest's mount window; false for other windows.
    pub(super) fn lan_mount_click(&mut self, conn: u32, click: &mut GuestMenuClick<'_>) -> bool {
        let p = click.packet;
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let Some((mob, columns)) = host
            .play
            .get(&conn)
            .and_then(|pl| pl.mount_window.as_ref())
            .filter(|m| m.window == p.container)
            .map(|m| (m.mob, m.columns))
        else {
            return false;
        };
        // handleContainerClick: a menu that is no longer `stillValid` takes no click. A mount
        // that died this tick has dropped its inventory already while its body still holds it;
        // the menu itself closes in `tick_lan_mount_menus`.
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
            && self.lan_mount_valid(&host.hub.guests[gi], mob)
            && let Some(index) = self.mobs.iter().position(|m| m.id == mob)
        {
            let mut menu = self.mount_engine_menu(index, columns);
            let result = self.try_guest_menu_op(gi, &mut menu, click.drop_pose, |menu, env| {
                click.apply(menu, env, DropMenu::Mount)
            });
            if result == Some(Err(())) {
                return true;
            }
            self.lan_mount_store(index, columns, &menu);
        }
        // The client is told what the server has, whatever it predicted.
        self.lan_resync_mount(conn);
        true
    }

    pub(super) fn lan_resync_mount(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if let Some(gm) = host
            .play
            .get_mut(&conn)
            .and_then(|pl| pl.mount_window.as_mut())
        {
            gm.sent = None;
        }
        self.lan_send_mount(conn);
    }

    /// The engine menu's mount slots back on mount `index`.
    fn lan_store_slots(menu: &Menu, columns: usize) -> Vec<Option<ItemStack>> {
        (0..columns * 3)
            .map(|k| menu.slot(2 + k).cloned())
            .collect()
    }
    fn lan_mount_store(&mut self, index: usize, columns: usize, menu: &Menu) {
        let (_, body, _) = self.mount_contents(index);
        let body = body.map(|_| menu.slot(1).cloned());
        let items = Self::lan_store_slots(menu, columns);
        self.mount_store(index, menu.slot(0).cloned(), body, items);
    }

    /// `AbstractContainerMenu.removed` of the guest's mount menu (`window`;
    /// None closes it whichever it is): the carried stack goes back to the
    /// inventory. False when no such window is open.
    pub(super) fn lan_mount_closed(&mut self, conn: u32, window: Option<i32>) -> bool {
        let Some(host) = self.lan.as_mut() else {
            return false;
        };
        let closed = host
            .play
            .get_mut(&conn)
            .and_then(|p| {
                p.mount_window
                    .take_if(|m| window.is_none_or(|w| m.window == w))
            })
            .is_some();
        if closed
            && let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && let Some(carried) = host.hub.guests.update(gi, |g| g.carried.take())
        {
            self.lan_place_back(gi, vec![carried]);
        }
        if closed && window.is_some() {
            self.player_menu_closing(conn);
        }
        closed
    }

    /// `AbstractMountInventoryMenu.stillValid`: the mount is alive and the
    /// guest's eyes are within the entity interaction range (3, 5 in creative)
    /// plus 4 of its box (`Player.isWithinEntityInteractionRange(mount, 4.0)`).
    pub(super) fn lan_mount_valid(&self, g: &horizon_client::lan::guest::Guest, mob: u32) -> bool {
        self.mobs
            .iter()
            .find(|m| m.id == mob)
            .is_some_and(|m| m.alive() && guest_within_entity_range(g, &m.bounds(), 4.0))
    }

    /// Each tick: a mount menu whose mount died or is out of reach closes
    /// (`AbstractMountInventoryMenu.stillValid`: alive, within the entity
    /// interaction range plus 4 of the eyes), and the others follow the
    /// mount's and the inventory's slots. Only the level the player is in
    /// checks its menu (`ServerPlayer.tick`): another level ticking has
    /// none of its mobs (#5331).
    pub(super) fn tick_lan_mount_menus(&mut self) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let open: Vec<(u32, u32, i32)> = host
            .play
            .iter()
            .filter_map(|(c, p)| p.mount_window.as_ref().map(|m| (*c, m.mob, m.window)))
            .collect();
        for (conn, mob, window) in open {
            let Some(host) = self.lan.as_ref() else {
                return;
            };
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            if !host.hub.here(g) {
                continue;
            }
            if self.lan_mount_valid(g, mob) {
                self.lan_send_mount(conn);
            } else {
                self.lan_mount_closed(conn, Some(window));
                if let Some(host) = self.lan.as_mut() {
                    host.hub.send_to(
                        conn,
                        &C::ContainerClose(play::ContainerClose { container: window }),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "daily"]
    fn mount_inventory_open_checks_the_mount_position_before_replacing_a_menu() {
        use horizon_client::mobs::Mob;
        use horizon_core::geometry::Vec3;
        use mod_api::{ContainerOpenedEvent, Flow, Hooks, ModEntry, Mods, ServerHost};
        use std::sync::{Arc, Mutex};

        struct Guard(Arc<Mutex<Vec<[i32; 3]>>>);

        #[mod_api::__subscriptions]
        impl Hooks for Guard {
            fn container_opened(
                &self,
                _: &mut dyn ServerHost,
                event: &mut ContainerOpenedEvent<'_>,
            ) -> Flow {
                assert_eq!(event.kind, "minecraft:horse_inventory");
                let pos = event.pos.expect("the mount inventory has a world location");
                self.0.lock().unwrap().push(pos);
                if pos[0] < 0 {
                    Flow::Cancel
                } else {
                    Flow::Continue
                }
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let mods: Mods = vec![ModEntry::new(
            "mount-area-guard",
            Box::new(Guard(heard.clone())),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
                for (id, x) in [(39, 0.25), (40, -0.25)] {
                    let mut mob = Mob::new(id, kind.clone(), Vec3::new(x, 64.0, 6.0), 0.0);
                    mob.extra.tame = true;
                    game.mobs.push(mob);
                }
                game.lan_open_mount(12, 39);
                game.lan_open_mount(12, 40);
                let open = game.lan.as_ref().unwrap().play[&12]
                    .mount_window
                    .as_ref()
                    .unwrap();
                assert_eq!(open.mob, 39);
                assert_eq!(*heard.lock().unwrap(), [[0, 64, 6], [-1, 64, 6]]);
            });
        });
    }

    /// `handleContainerClick` refuses a click on a menu that is no longer `stillValid`: a
    /// chested donkey that died this tick has already dropped its chest, so its body's slots
    /// must not be taken as well.
    #[test]
    #[ignore = "daily"]
    fn a_dead_mounts_inventory_takes_no_click() {
        use horizon_client::{items::ItemStack, mobs::Mob};
        use horizon_core::geometry::Vec3;
        use horizon_protocol::packets::play;
        super::super::native_guest_turn_tests::with_guests(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:donkey")
                .unwrap()
                .clone();
            let mut mob = Mob::new(40, kind, Vec3::new(3.0, 64.0, 6.0), 0.0);
            mob.extra.tame = true;
            mob.extra.chested = true;
            mob.extra.items = vec![None; 15];
            mob.extra.items[0] = Some(ItemStack::new("minecraft:diamond", 5));
            mob.extra.items[1] = Some(ItemStack::new("minecraft:emerald", 7));
            game.mobs.push(mob);
            game.lan_open_mount(12, 40);
            let window = game.lan.as_ref().unwrap().play[&12]
                .mount_window
                .as_ref()
                .unwrap()
                .window;
            let shift_click = |slot| {
                play::Serverbound::ContainerClick(play::ContainerClick {
                    container: window,
                    state: 0,
                    slot,
                    button: 0,
                    input: 1,
                    changed: vec![],
                    carried: None,
                })
            };
            let held = |game: &crate::game::Game<'_>, item: &str| -> u32 {
                let guest = game.lan.as_ref().unwrap().hub.guest(12).unwrap();
                guest
                    .inventory
                    .iter()
                    .flatten()
                    .filter(|s| &*s.item == item)
                    .map(|s| s.count)
                    .sum()
            };
            // While it lives, a shift-click takes the chest's first slot (menu slot 2).
            game.lan_guest_packet(12, shift_click(2));
            assert_eq!(held(game, "minecraft:diamond"), 5);
            assert_eq!(game.mobs[0].extra.items[0], None);
            // Dead, with the menu still open: the next slot stays on the body.
            game.mobs[0].health = 0.0;
            game.lan_guest_packet(12, shift_click(3));
            assert_eq!(held(game, "minecraft:emerald"), 0);
            assert_eq!(
                game.mobs[0].extra.items[1],
                Some(ItemStack::new("minecraft:emerald", 7))
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn horse_inventory_click_keeps_complete_body_stack() {
        use horizon_client::{items::ItemStack, menu::Input, mobs::Mob};
        use horizon_core::geometry::Vec3;
        super::super::native_entity_refs::tests::with_game(|game| {
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut mob = Mob::new(12, kind, Vec3::ZERO, 0.0);
            mob.extra.tame = true;
            let mut armor = ItemStack::new("minecraft:iron_horse_armor", 3);
            armor.damage = 7;
            armor
                .data_mut()
                .enchantments
                .push(("minecraft:unbreaking".into(), 2));
            mob.extra.set_body_stack(Some(armor.clone()));
            game.mobs.push(mob);
            game.open_mount_menu(0);
            let mut menu = game.mount_engine_menu(0, 0);
            assert_eq!(menu.slot(1), Some(&armor));
            let env = game.menu_env();
            let _ = menu.click(&env, 2, 0, Input::Pickup);
            game.lan_mount_store(0, 0, &menu);
            assert_eq!(game.mobs[0].equipment_stacks()[6], Some(armor));
        });
    }
}
