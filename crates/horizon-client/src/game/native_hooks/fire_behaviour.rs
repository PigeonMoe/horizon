use super::*;
use crate::game::commands::Target;

impl Game<'_> {
    fn checks_block_ignition(&self) -> bool {
        self.native_authoritative()
            && (mod_api::__private::listened(mod_api::HookId::BlockIgnite)
                || mod_api::__private::listened(mod_api::HookId::PlayerBlockIgnitePre))
    }
    pub(in crate::game) fn player_block_ignite_allowed(
        &mut self,
        pos: [i32; 3],
        cause: &'static str,
    ) -> bool {
        if !self.checks_block_ignition() {
            return true;
        }
        let player = (!self.lan_acting_fake)
            .then(|| self.player_uuid_of(self.lan_acting))
            .flatten()
            .map(horizon_client::lan::playerdata::ints_uuid);
        self.with_level(|_, level, _| {
            level.block_ignite_allowed_by(BlockPos::new(pos[0], pos[1], pos[2]), cause, player)
        })
    }

    /// Resolves a burning arrow's real player owner in this level. Collision
    /// can temporarily represent a remote shooter as a mob entity ID.
    pub(in crate::game) fn arrow_block_player_owner(
        &self,
        owner_uuid: Option<[i32; 4]>,
    ) -> Option<(u128, Option<u32>)> {
        let uuid = horizon_client::lan::playerdata::ints_uuid(owner_uuid?);
        match self.target_by_uuid(uuid) {
            Some(Target::Player) if self.player.is_some() && !self.lan_acting_fake => {
                Some((uuid, self.lan_acting))
            }
            Some(Target::Guest(conn))
                if self
                    .command_guest(conn)
                    .is_some_and(|guest| guest.placed_in(self.dim.id())) =>
            {
                Some((uuid, Some(conn)))
            }
            _ => None,
        }
    }

    /// Paper resolves a burning arrow's owner for BlockIgniteEvent.
    pub(in crate::game) fn arrow_block_ignite_allowed(
        &mut self,
        pos: BlockPos,
        kind: &str,
        owner_uuid: Option<[i32; 4]>,
    ) -> bool {
        if !self.checks_block_ignition() {
            return true;
        }
        let player = self
            .arrow_block_player_owner(owner_uuid)
            .map(|(uuid, _)| uuid);
        // The Paper cause switch uses ARROW for a plain arrow and its
        // FLINT_AND_STEEL fallback for a spectral arrow.
        let cause = if kind == "minecraft:arrow" {
            "arrow"
        } else {
            "flint_and_steel"
        };
        self.with_level(|_, level, _| level.block_ignite_allowed_by(pos, cause, player))
    }

    /// Replace a checked air block with fire, after reporting the actual ignition source.
    pub(in crate::game) fn ignite_block_from(
        &mut self,
        pos: BlockPos,
        cause: &'static str,
        player: Option<u128>,
    ) -> horizon_client::Result<()> {
        let state = self.with_level(|behaviour, level, ticks| {
            level
                .block_ignite_allowed_by(pos, cause, player)
                .then(|| behaviour.fire_state_for(level, ticks, pos))
        });
        if let Some(state) = state {
            self.level_set([pos.x, pos.y, pos.z], state)?;
        }
        Ok(())
    }
}

/// The level's side of fire and lava ignition (`FireBlock`, `LavaFluid`), which `GameLevel`'s
/// `LevelAccess` forwards to: the mods' answers on an authoritative server, and the level's own
/// facts. A client's copy of the level asks no mod.
impl GameLevel<'_> {
    /// `allow_block_ignite`: the `block_ignite` hook's cancel.
    pub(in crate::game) fn block_ignite_allowed(&self, pos: BlockPos, cause: &'static str) -> bool {
        self.block_ignite_allowed_by(pos, cause, None)
    }

    fn block_ignite_allowed_by(
        &self,
        pos: BlockPos,
        cause: &'static str,
        player: Option<u128>,
    ) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_ignite(self.dimension, player, pos, cause, &mut cancel);
            if !cancel
                && mod_api::__private::listened(mod_api::HookId::PlayerBlockIgnitePre)
                && let Some(player) = player
                && let Some((name, permission_level)) = self
                    .block_trigger_player
                    .filter(|source| source.uuid == Some(player))
                    .map(|source| (source.name, source.permission_level))
                    .or_else(|| {
                        let hub = self.lan_hub?;
                        let guest = hub
                            .guests
                            .simulation_iter()
                            .find(|guest| guest.uuid == player)?;
                        Some((guest.name.as_str(), guest.permission))
                    })
            {
                self.native_player_block_ignite_pre(mod_api::__private::PlayerBlockIgnitePreCall {
                    dimension: self.dimension,
                    pos,
                    cause,
                    player,
                    name,
                    permission_level,
                    cancel: &mut cancel,
                });
            }
        }
        !cancel
    }

    /// `allow_block_burn`: the `block_burn` hook's cancel.
    pub(in crate::game) fn block_burn_allowed(&self, pos: BlockPos) -> bool {
        let mut cancel = false;
        if self.native_authoritative {
            self.native_block_burn(self.dimension, pos, &mut cancel);
        }
        !cancel
    }

    /// `before_fire_odds`: the block's parts answer `fire_odds`.
    pub(in crate::game) fn answer_fire_odds(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        odds: &mut Option<(i32, i32)>,
    ) {
        if self.native_authoritative {
            self.native_fire_odds(self.dimension, state, pos, face, odds);
        }
    }

    /// `before_is_fire_source`: the block's parts answer `is_fire_source`.
    pub(in crate::game) fn answer_is_fire_source(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        source: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_is_fire_source(self.dimension, state, pos, source);
        }
    }

    /// `before_ignited_by_lava`: the block's parts answer `ignited_by_lava`.
    pub(in crate::game) fn answer_ignited_by_lava(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
        ignites: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_ignited_by_lava(self.dimension, state, pos, face, ignites);
        }
    }

    /// `on_caught_fire`: the burned block's parts hear `on_caught_fire`.
    pub(in crate::game) fn caught_fire(
        &mut self,
        ticks: &mut Ticks,
        state: BlockStateId,
        pos: BlockPos,
        face: Direction,
    ) {
        if self.native_authoritative {
            self.native_on_caught_fire(ticks, self.dimension, state, pos, face);
        }
    }

    /// `before_smoke_source`: the block's parts answer `smoke_source`.
    pub(in crate::game) fn answer_smoke_source(
        &self,
        state: BlockStateId,
        smoke: &mut Option<bool>,
    ) {
        if self.native_authoritative {
            self.native_smoke_source(self.dimension, state, smoke);
        }
    }

    /// `ServerLevel.canSpreadFireAround`: a player (not a spectator) closer
    /// than the `fire_spread_radius_around_player` rule to the block's corner
    /// (`ChunkMap.anyPlayerCloseEnoughTo`); -1 spreads everywhere.
    pub(in crate::game) fn player_near_fire(&self, pos: BlockPos) -> bool {
        let radius = self.rules.fire_spread_radius_around_player;
        radius == -1
            || self.fire_players.iter().any(|p| {
                let d = [
                    p[0] - f64::from(pos.x),
                    p[1] - f64::from(pos.y),
                    p[2] - f64::from(pos.z),
                ];
                d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < f64::from(radius) * f64::from(radius)
            })
    }

    /// The `INCREASED_FIRE_BURNOUT` environment attribute of the biome at `pos`.
    pub(in crate::game) fn biome_burns_out_fire(&self, pos: BlockPos) -> bool {
        self.world
            .chunks
            .get(&(pos.x >> 4, pos.z >> 4))
            .is_some_and(|chunk| {
                self.biomes
                    .get(chunk.biome(pos.x, pos.y, pos.z))
                    .increased_fire_burnout
            })
    }
}
