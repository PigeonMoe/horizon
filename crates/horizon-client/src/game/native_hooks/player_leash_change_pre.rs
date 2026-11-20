use super::*;
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::play::{Clientbound, SetEntityLink};

impl Game<'_> {
    pub(in crate::game) fn native_player_leash_change_pre(
        &self,
        player: u128,
        entity: u32,
        dimension: &str,
        position: [f64; 3],
        attach: bool,
        cancel: &mut bool,
    ) {
        self.fire_player_leash_change_pre(player, entity, dimension, position, attach, cancel);
    }

    pub(in crate::game) fn player_leash_change_allowed(
        &mut self,
        entity: u32,
        position: Vec3,
        attach: bool,
    ) -> bool {
        if !self.native_authoritative()
            || !mod_api::__private::listened(mod_api::HookId::PlayerLeashChangePre)
        {
            return true;
        }
        let player = self.native_acting_id();
        let mut cancel = false;
        self.native_player_leash_change_pre(
            player,
            entity,
            self.dim.id(),
            position.components(),
            attach,
            &mut cancel,
        );
        if cancel {
            self.resend_player_leash(entity);
        }
        !cancel
    }

    fn resend_player_leash(&self, entity: u32) {
        let (Some(conn), Some(host)) = (self.lan_acting, self.lan.as_ref()) else {
            return;
        };
        let source: &dyn mod_api::Host = self;
        let holder = source
            .entity(entity)
            .and_then(|entity| entity.leash_holder(self));
        let dest = match holder {
            Some(mod_api::LeashHolder::Entity(holder)) => holder.id() as i32,
            Some(mod_api::LeashHolder::Knot(fence)) => {
                self.knot_ids.get(&fence).copied().unwrap_or_default() as i32
            }
            _ => 0,
        };
        host.hub.send_to(
            conn,
            &Clientbound::SetEntityLink(SetEntityLink {
                source: entity as i32,
                dest,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::commands::Target;
    use horizon_client::lan::{Out, PlayerSender, playerdata::uuid_ints};
    use horizon_client::mobs::leash::Holder;
    use mod_api::{Flow, Hooks, Host, ModEntry, PlayerLeashChangePreEvent};
    use std::sync::{Arc, Mutex, mpsc};

    type Change = (u128, u32, String, [f64; 3], bool);

    struct ProtectEast(Arc<Mutex<Vec<Change>>>);

    #[mod_api::__subscriptions]
    impl Hooks for ProtectEast {
        fn player_leash_change_pre(
            &self,
            _: &dyn Host,
            event: &mut PlayerLeashChangePreEvent<'_>,
        ) -> Flow {
            self.0.lock().unwrap().push((
                event.player,
                event.entity,
                event.dimension.into(),
                event.position,
                event.attach,
            ));
            if event.position[0] >= 2.0 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    #[test]
    #[ignore = "daily"]
    fn knot_ties_and_cuts_respect_each_mob_and_boat_without_detaching_denied_leads() {
        crate::entry::install_client_hooks();
        let seen = Arc::new(Mutex::new(Vec::new()));
        mod_platform::testing::with_mods(
            vec![ModEntry::new(
                "protect_east",
                Box::new(ProtectEast(seen.clone())),
            )]
            .into(),
            || {
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.build_registries();
                    crate::game::native_player_death::tests::add_guest(game);
                    let (tx, rx) = mpsc::channel();
                    game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
                        guest.sender = PlayerSender::new(tx, Arc::default());
                        guest.position = [1.5, 70.0, 1.5];
                        guest.loaded = true;
                    });
                    game.summon_with_nbt("minecraft:pig", [1.5, 70.25, 1.5], "{}")
                        .unwrap();
                    game.summon_with_nbt("minecraft:pig", [2.5, 70.25, 1.5], "{}")
                        .unwrap();
                    game.summon_with_nbt("minecraft:oak_boat", [2.75, 70.25, 1.5], "{}")
                        .unwrap();
                    let west = game.mobs[0].id;
                    let east = game.mobs[1].id;
                    let boat = game.boats[0].id;
                    let fence = [1, 70, 1];
                    let mut knot = 0;

                    game.as_guest(1, |game| {
                        for target in [Target::Mob(west), Target::Mob(east), Target::Boat(boat)] {
                            assert!(game.set_leash(target, Holder::Player, Some(uuid_ints(123))));
                        }
                        assert!(game.use_knot(fence, false));
                        assert_eq!(game.mobs[0].extra.leash, Some(Holder::Knot(fence)));
                        assert_eq!(game.mobs[1].extra.leash, Some(Holder::Player));
                        assert_eq!(game.boats[0].leash, Some(Holder::Player));
                        knot = game.knot_ids[&fence];

                        assert!(game.set_leash(Target::Mob(east), Holder::Knot(fence), None));
                        assert!(game.set_leash(Target::Boat(boat), Holder::Knot(fence), None));
                        *game.held_mut() = Some(ItemStack::new("minecraft:shears", 1));
                        let before = game.held().cloned();
                        assert!(game.use_knot(fence, true));
                        let after_one_cut = game.held().cloned();
                        assert_ne!(before, after_one_cut);
                        assert_eq!(game.mobs[0].extra.leash, None);
                        assert_eq!(game.mobs[1].extra.leash, Some(Holder::Knot(fence)));
                        assert_eq!(game.boats[0].leash, Some(Holder::Knot(fence)));
                        assert_eq!(game.knot_ids[&fence], knot);
                        assert!(!game.use_knot(fence, true));
                        assert_eq!(game.held().cloned(), after_one_cut);
                    })
                    .unwrap();

                    let calls = seen.lock().unwrap();
                    assert!(
                        calls
                            .iter()
                            .all(|change| change.0 == 123 && change.2 == game.dim.id())
                    );
                    for (id, x) in [(west, 1.5), (east, 2.5), (boat, 2.75)] {
                        for attach in [false, true] {
                            assert!(calls.iter().any(|change| change.1 == id
                                && change.3 == [x, 70.25, 1.5]
                                && change.4 == attach));
                        }
                    }
                    let packets = rx.try_iter().flat_map(|out| match out {
                        Out::Packet(packet) => vec![packet],
                        Out::Packets(packets) => packets,
                        Out::Disconnect(_) => panic!("denied cut disconnected the guest"),
                    });
                    assert!(packets.into_iter().any(|packet| {
                    let mut reader = horizon_protocol::buf::Reader::new(&packet);
                    let kind = reader.varint().unwrap();
                    matches!(Clientbound::decode(kind, reader.rest()).unwrap(),
                        Clientbound::SetEntityLink(link) if link.source == east as i32 && link.dest == knot as i32)
                }));
                })
            },
        );
    }
}
