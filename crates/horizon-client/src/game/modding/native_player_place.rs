//! A block a mod writes in a player's turn: `level_set_block` with a placement's flags, then `setPlacedBy`.
use super::{Game, native_server::PlayerRead};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use mod_api::{
    BlockStateError, BlockUpdateFlags, LevelRef, LevelWriteError, PlayerRef,
    PlayerWorldRefusal as Refusal,
};

impl Game<'_> {
    pub(super) fn level_replace_block_for_player(
        &mut self,
        level: LevelRef,
        player: PlayerRef,
        pos: [i32; 3],
        expected: BlockStateId,
        replacement: BlockStateId,
        flags: BlockUpdateFlags,
    ) -> Result<bool, LevelWriteError> {
        self.native_level_world_result(level)?;
        if self.dim.key() != level.dimension() {
            return Err(LevelWriteError::Unavailable);
        }
        let registry = &self.pack.assets.registry;
        if registry.state(expected).is_none() || registry.state(replacement).is_none() {
            return Err(BlockStateError::UnknownState.into());
        }
        let [x, y, z] = pos;
        let current = |game: &Self| {
            game.native_level_write_allowed(level, pos) && game.world.block(x, y, z) == expected
        };
        let allowed_player = |game: &Self| {
            let in_level = match game.player_handle_read(player) {
                Some(PlayerRead::Acting { mode, body, .. }) => {
                    mode != 3 && body.is_some_and(|body| body.health.health() > 0.0)
                }
                Some(PlayerRead::Guest(guest)) => {
                    !guest.dead && guest.game_mode != 3 && guest.dimension == game.dim.id()
                }
                None => false,
            };
            in_level
                && !game.lan.as_ref().is_some_and(|host| {
                    game.spawn_protection_for(host, player.uuid())
                        .is_some_and(|radius| game.spawn_protects(radius, game.dim.id(), pos))
                })
        };
        if !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
            || !current(self)
            || !allowed_player(self)
            || self.landing_change_cancelled(
                Some(player.uuid()),
                BlockPos::new(x, y, z),
                expected,
                replacement,
            )
            || !current(self)
            || !allowed_player(self)
        {
            return Ok(false);
        }
        if expected == replacement {
            return Ok(true);
        }
        self.level_set_block(level, pos, replacement, flags)
    }

    /// The one block write (`level_set_block`, its block entity included) with a placement's
    /// flags, then `Block.setPlacedBy` in the player's turn; nothing else of `BlockItem.place`.
    pub(super) fn player_place_block(
        &mut self,
        player: PlayerRef,
        level: LevelRef,
        pos: [i32; 3],
        state: BlockStateId,
        update_neighbour_shapes: bool,
    ) -> Result<(), Refusal> {
        if !self.is_logical_server() {
            return Err(Refusal::Client);
        }
        let dead = match self.player_handle_read(player).ok_or(Refusal::Stale)? {
            PlayerRead::Acting { .. } => self
                .player
                .as_ref()
                .is_none_or(|p| p.health.health() <= 0.0),
            PlayerRead::Guest(g) => g.dead,
        };
        if dead {
            return Err(Refusal::Dead);
        }
        if self.pack.assets.registry.state(state).is_none() {
            return Err(Refusal::UnknownState);
        }
        if self.native_level_world(level).is_none() {
            return Err(Refusal::StaleLevel);
        }
        if level.dimension() != self.dim.key() {
            return Err(Refusal::OtherLevel);
        }
        let [x, y, z] = pos;
        if !self.world.shape.contains_y(y)
            || !(-30_000_000..30_000_000).contains(&x)
            || !(-30_000_000..30_000_000).contains(&z)
        {
            return Err(Refusal::OutOfWorld { pos });
        }
        if !self.world.loaded(x, z) {
            return Err(Refusal::NotLoaded { pos });
        }
        if self.world.block(x, y, z) == state {
            return Err(Refusal::Unchanged);
        }
        let mut flags = BlockUpdateFlags::NEIGHBORS | BlockUpdateFlags::CLIENTS;
        if !update_neighbour_shapes {
            flags = flags | BlockUpdateFlags::KNOWN_SHAPE;
        }
        // The write is `level_set_block`'s, on the level as it stands (a guest's turn is not
        // the authoritative level); only `setPlacedBy` runs in the player's turn.
        match self.level_set_block(level, pos, state, flags) {
            Ok(true) => {}
            Ok(false) => return Err(Refusal::Unchanged),
            Err(_) => return Err(Refusal::Stale),
        }
        let at = BlockPos::new(x, y, z);
        let ran = self.native_as_player_handle(player, |game| {
            game.with_level(|b, level, ticks| b.set_placed_by_state(level, ticks, at, state));
            let _ = game.process_level_events();
        });
        if ran { Ok(()) } else { Err(Refusal::Stale) }
    }
}
