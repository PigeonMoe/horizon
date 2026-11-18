//! A hub container click the game owns. `Hub::container_click` used to run inside
//! `self.lan.as_mut()`, so nothing it called could reach the game: a hook fired in the middle
//! of a click had no `&Game` to be given. Here the game lifts the host out of itself for the
//! click, so the click code and the game are separate values and the game is whole while the
//! hub runs.
use super::*;
use horizon_client::lan::menu::input;

impl Game<'_> {
    /// Run `f` with the LAN host taken out of `self.lan`, then put it back. `f` is given the whole
    /// game (`self.lan` is `None` while it runs) and the host. The host is restored on every
    /// path, including a panic in `f`, which is then propagated: no borrow of the host outlives
    /// the call and the game is not left without it. `None` when there is no host to lift,
    /// which includes a call made while the host is already lifted.
    pub(in crate::game) fn with_lan_lifted<R>(
        &mut self,
        f: impl FnOnce(&mut Self, &mut LanHost) -> R,
    ) -> Option<R> {
        let mut host = self.lan.take()?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(self, &mut host)));
        debug_assert!(self.lan.is_none(), "a lifted LAN host was replaced");
        self.lan = Some(host);
        match result {
            Ok(result) => Some(result),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// `ServerGamePacketListenerImpl.handleContainerClick` for the guest `conn`: the hub's
    /// click, run with the host lifted out of the game. Returns the stacks the click threw out.
    pub(super) fn lan_hub_click(&mut self, conn: u32, p: &play::ContainerClick) -> Vec<ItemStack> {
        let check_drop = self.native_authoritative()
            && (p.input == input::THROW || (p.input == input::PICKUP && p.slot == -999))
            && mod_api::__private::listened(mod_api::HookId::PlayerDropPre);
        match self.run_hub_click(conn, p, check_drop) {
            Ok(drops) => drops,
            Err(()) => {
                if !self.player_drop_allowed(Some(conn)) {
                    if let Some(host) = &mut self.lan {
                        host.hub.resend_menu(conn, p.container, &self.world);
                    }
                    return Vec::new();
                }
                // Hooks have a mutable server host. Read the menu and world again
                // so their edits are not overwritten by a pre-hook snapshot.
                self.run_hub_click(conn, p, false).unwrap_or_default()
            }
        }
    }

    fn run_hub_click(
        &mut self,
        conn: u32,
        p: &play::ContainerClick,
        check_drop: bool,
    ) -> Result<Vec<ItemStack>, ()> {
        self.with_lan_lifted(|game, host| {
            // The game is lent to the click as the mods' `menu_slot_rules` (their read-only `Host`).
            // `self.lan` is `None` meanwhile: what a hook reads of the LAN host answers as if none
            // were open.
            let rules = ItemRules(
                world_items!(game),
                &world_stations!(game).enchantments,
                Some(&*game),
            );
            let Some(mut prepared) = host.hub.prepare_click(conn, p, &game.world) else {
                return Ok(Vec::new());
            };
            if check_drop && host.hub.would_manually_drop(&mut prepared, p, &rules) {
                return Err(());
            }
            // The game is whole here: `game` can be read or written while the hub clicks.
            let outcome = host.hub.run_click(&mut prepared, p, &rules);
            Ok(host.hub.finish_click(prepared, p, &mut game.world, outcome))
        })
        .unwrap_or_else(|| Ok(Vec::new()))
    }
}
