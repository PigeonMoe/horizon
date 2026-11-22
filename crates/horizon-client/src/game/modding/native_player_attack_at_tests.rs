//! Location admission uses the same authoritative guest and PvP rule as combat.
use super::native_guest_turn_tests::with_guests;
use horizon_client::gamerules::Value;
use mod_api::{
    DimensionKey, Flow, Hooks, Host, LevelRef, ModEntry, PlayerAttackAtPreEvent, WorldEpoch,
};
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "daily"]
fn location_pvp_checks_pvp_and_dispatches_only_loaded_targets_for_the_live_player() {
    const PROTECTED: [i32; 3] = [4, 70, 2];
    const OPEN: [i32; 3] = [8, 70, 2];
    struct Claims(Arc<Mutex<Vec<[i32; 3]>>>);

    #[mod_api::__subscriptions]
    impl Hooks for Claims {
        fn player_attack_at_pre(
            &self,
            host: &dyn Host,
            event: &mut PlayerAttackAtPreEvent<'_>,
        ) -> Flow {
            assert_eq!(event.dimension, "minecraft:overworld");
            assert_eq!(host.player(13), Some(event.player));
            self.0.lock().unwrap().push(event.pos);
            if event.pos == PROTECTED {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    let calls = Arc::new(Mutex::new(Vec::new()));
    mod_platform::testing::with_mods(
        vec![ModEntry::new(
            "location-claims-fixture",
            Box::new(Claims(calls.clone())),
        )]
        .into(),
        || {
            with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.options.execution_role =
                    crate::game::execution_role::ExecutionRole::DedicatedServer;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                assert!(game.ui.rules.set("pvp", Value::Bool(false)));
                let player = game.player(13).unwrap();
                let level = game.level(game.dim.id()).unwrap();
                assert!(!level.allow_player_attack_at(game, player, OPEN));
                assert!(game.ui.rules.set("pvp", Value::Bool(true)));
                assert!(level.allow_player_attack_at(game, player, OPEN));
                assert!(!level.allow_player_attack_at(game, player, PROTECTED));
                assert!(!level.allow_player_attack_at(game, player, [4, -1_000, 2]));
                assert!(!level.allow_player_attack_at(game, player, [1_000, 70, 2]));
                let stale =
                    LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
                assert!(!stale.allow_player_attack_at(game, player, OPEN));
                let other = LevelRef::new(level.epoch(), DimensionKey::NETHER);
                assert!(!other.allow_player_attack_at(game, player, OPEN));
                game.lan.as_mut().unwrap().hub.update_guest(13, |guest| {
                    guest.dimension = "minecraft:the_nether".into();
                });
                assert!(!level.allow_player_attack_at(game, player, OPEN));
            });
        },
    );
    assert_eq!(*calls.lock().unwrap(), [OPEN, PROTECTED]);
}
