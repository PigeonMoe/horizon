//! Resident-world landing, registered block reactions and fall-damage requests.
//! Health, sound/particle playback and network delivery remain event consumers.
use crate::{
    Result,
    collisions::SceneCollisions,
    motion::{MotionEvent, MotionWorld},
};
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::native_load_context::LoopContext;
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::{
        BlockLandingRegistry, BlockLandingWorld, DamageSourceOwner, FallDefinition, LandingDamage,
        LandingDefinition, LandingEntityKind, LandingEvent, LandingSound,
    },
    entity_obstruction::EntityIdentity,
    fall_damage::FallDamageWorld,
    living_fall::{self, LivingFallWorld},
    living_landing::LivingLandingWorld,
    movement_properties::MovementProperties,
    registry::BlockStateId,
};
use mod_api::{Compound, GameMode, ItemStack};
use std::{collections::BTreeMap, fs, path::Path};

/// Called on the server before a landing converts farmland to its base block.
#[inline(never)]
pub fn native_farmland_trample(
    context: &LoopContext,
    entity: u32,
    pos: [i32; 3],
    state: u32,
    distance: f64,
    cancel: &mut bool,
) {
    crate::native_client_hooks::farmland_trample(context, entity, pos, state, distance, cancel);
}

/// Call the footwear policy only while the action owns the worn feet item.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub fn native_player_footwear_reaction(
    context: &LoopContext,
    action: &str,
    actor: (u128, GameMode),
    boots: &ItemStack,
    persisted: Option<&Compound>,
    sound: &mut Option<String>,
    cancel: &mut bool,
) {
    crate::native_client_hooks::player_footwear_reaction(
        context, action, actor, boots, persisted, sound, cancel,
    );
}

#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub fn native_player_fall_reaction(
    context: &LoopContext,
    actor: (u128, GameMode),
    boots: &ItemStack,
    persisted: Option<&Compound>,
    damage: f32,
    effect: &mut (u128, Option<(String, f32)>),
    cancel: &mut bool,
) {
    crate::native_client_hooks::player_fall_reaction(
        context, actor, boots, persisted, damage, effect, cancel,
    );
}

#[inline(never)]
pub fn native_player_gliding_impact(
    context: &LoopContext,
    action: &str,
    actor: (u128, GameMode),
    helmet: &ItemStack,
    persisted: Option<&Compound>,
    reaction: &mut (Option<String>, bool),
    cancel: &mut bool,
) {
    crate::native_client_hooks::player_gliding_impact(
        context, action, actor, helmet, persisted, reaction, cancel,
    );
}

/// Read-only, short-lived references to the authoritative player's worn feet and saved data.
#[derive(Clone, Copy)]
pub struct PlayerFootwear<'a> {
    player: u128,
    mode: mod_api::GameMode,
    boots: &'a crate::items::ItemStack,
    persisted: Option<&'a horizon_core::nbt::Compound>,
}

impl<'a> PlayerFootwear<'a> {
    pub fn new(
        uuid: Option<u128>,
        mode: mod_api::GameMode,
        boots: Option<&'a crate::items::ItemStack>,
        saved: Option<&'a horizon_core::nbt::Tag>,
    ) -> Option<Self> {
        if !crate::native_client_hooks::player_footwear_listened()
            && !crate::native_client_hooks::player_fall_reaction_listened()
        {
            return None;
        }
        let player = uuid?;
        let boots = worn_stack(boots)?;
        let persisted = player_persisted(saved);
        Some(Self {
            player,
            mode,
            boots,
            persisted,
        })
    }

    /// A canceled action and its optional, registry-validated level sound.
    pub fn reaction(self, context: &LoopContext, action: &str) -> Option<Option<&'static str>> {
        use horizon_world::native_load_context::LoopSide;
        if context.side() != LoopSide::Server
            || !crate::native_client_hooks::player_footwear_listened()
        {
            return None;
        }
        let mut sound = None;
        let mut cancel = false;
        let boots = crate::crafting::mod_kinds::to_api(self.boots)?;
        native_player_footwear_reaction(
            context,
            action,
            (self.player, self.mode),
            &boots,
            self.persisted,
            &mut sound,
            &mut cancel,
        );
        cancel.then(|| registered_sound(context, sound.as_deref()))
    }

    pub(crate) fn fall_reaction(
        self,
        context: &LoopContext,
        damage: f32,
    ) -> Option<LandingReaction> {
        use horizon_world::native_load_context::LoopSide;
        if context.side() != LoopSide::Server
            || !crate::native_client_hooks::player_fall_reaction_listened()
        {
            return None;
        }
        let boots = crate::crafting::mod_kinds::to_api(self.boots)?;
        let mut effect = (0, None);
        let mut cancel = false;
        native_player_fall_reaction(
            context,
            (self.player, self.mode),
            &boots,
            self.persisted,
            damage,
            &mut effect,
            &mut cancel,
        );
        let (reaction, sound) = effect;
        if !cancel || reaction == 0 {
            return None;
        }
        let sound = sound.and_then(|(name, pitch)| {
            pitch.is_finite().then_some(())?;
            registered_sound(context, Some(&name)).map(|name| (name, pitch))
        });
        Some(LandingReaction::Effect {
            player: self.player,
            reaction,
            damage,
            sound,
        })
    }
}

fn worn_stack(stack: Option<&crate::items::ItemStack>) -> Option<&crate::items::ItemStack> {
    stack.filter(|stack| stack.count > 0 && &*stack.item != "minecraft:air")
}

fn player_persisted(saved: Option<&horizon_core::nbt::Tag>) -> Option<&Compound> {
    match saved?.get("NeoForgeData")?.get("PlayerPersisted")? {
        horizon_core::nbt::Tag::Compound(data) => Some(data),
        _ => None,
    }
}

fn registered_sound(context: &LoopContext, id: Option<&str>) -> Option<&'static str> {
    let id = id?;
    context
        .registry("minecraft:sound_event")?
        .iter()
        .any(|key| key == id)
        .then(|| crate::mobs::entity_sounds::intern_full_id(id))
}

/// Equipment belonging to the player whose movement is executing now.
pub struct LandingEquipment<'a> {
    boots: Option<PlayerFootwear<'a>>,
    helmet: Option<FlightHelmet<'a>>,
    origin: Option<Vec3>,
    check_environment_damage: bool,
    paused_fall: Option<Box<PausedFallDamage>>,
}

/// A landing hit admitted by vanilla, incoming damage listeners and the player's equipment.
/// The hit borrows nothing, so its continuation can release the movement borrow for the host.
pub struct PausedFallDamage {
    pub damage: f32,
    pub position: Vec3,
    pub event_index: usize,
    pub hit: crate::health::PausedHurt,
}

struct FlightHelmet<'a> {
    actor: (u128, GameMode),
    persisted: Option<&'a Compound>,
    slot: &'a mut Option<crate::items::ItemStack>,
    just_stopped_gliding: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct HelmetSource<'a> {
    actor: (u128, GameMode),
    persisted: Option<&'a Compound>,
    item: &'a crate::items::ItemStack,
}

#[derive(Clone, Copy, Default)]
#[doc(hidden)]
pub enum LandingReaction {
    #[default]
    None,
    Footwear(Option<&'static str>),
    Helmet {
        sound: Option<&'static str>,
        wear: bool,
    },
    Effect {
        player: u128,
        reaction: u128,
        damage: f32,
        sound: Option<(&'static str, f32)>,
    },
}

impl LandingReaction {
    /// Describe the equipment's deferred level work at the accepted impact position.
    pub fn effect_at(self, at: Vec3, dimensions: crate::pose::Dimensions) -> Option<MotionEvent> {
        let Self::Effect {
            player,
            reaction,
            damage,
            sound,
        } = self
        else {
            return None;
        };
        let bounds = dimensions.bounds(at);
        use horizon_core::geometry::Axis;
        let axes = [Axis::X, Axis::Y, Axis::Z];
        Some(MotionEvent::Fall(FallEvent::EquipmentEffect {
            player,
            reaction,
            origin: [at.x, at.y, at.z],
            bounds: [
                axes.map(|axis| bounds.min(axis)),
                axes.map(|axis| bounds.max(axis)),
            ],
            eye: [at.x, at.y + f64::from(dimensions.eye_height), at.z],
            damage,
            sound,
        }))
    }
}

impl<'a> LandingEquipment<'a> {
    pub fn new(
        uuid: Option<u128>,
        mode: GameMode,
        armor: &'a mut [Option<crate::items::ItemStack>; 4],
        saved: Option<&'a horizon_core::nbt::Tag>,
        just_stopped_gliding: bool,
        check_environment_damage: bool,
    ) -> Option<Self> {
        let [head, _, _, feet] = armor;
        let boots = PlayerFootwear::new(uuid, mode, feet.as_ref(), saved);
        let helmet = if crate::native_client_hooks::player_gliding_impact_listened() {
            uuid.map(|uuid| FlightHelmet {
                actor: (uuid, mode),
                persisted: player_persisted(saved),
                slot: head,
                just_stopped_gliding,
            })
        } else {
            None
        };
        let check_environment_damage = uuid.is_some() && check_environment_damage;
        (boots.is_some() || helmet.is_some() || check_environment_damage).then_some(Self {
            boots,
            helmet,
            origin: None,
            check_environment_damage,
            paused_fall: None,
        })
    }

    pub fn take_paused_fall(&mut self) -> Option<PausedFallDamage> {
        self.paused_fall.take().map(|fall| *fall)
    }

    /// Position at the end of the physical move being replayed. The authoritative
    /// player's position can already be at the end of a later move in that batch.
    pub fn set_fall_origin(&mut self, origin: Vec3) {
        self.origin = Some(origin);
    }

    fn helmet_at_impact(&self, gliding: bool) -> Option<HelmetSource<'_>> {
        let helmet = self.helmet.as_ref()?;
        if !gliding && !helmet.just_stopped_gliding {
            return None;
        }
        Some(HelmetSource {
            actor: helmet.actor,
            persisted: helmet.persisted,
            item: worn_stack(helmet.slot.as_ref())?,
        })
    }
}

impl HelmetSource<'_> {
    pub(crate) fn reaction(self, context: &LoopContext, action: &str) -> Option<LandingReaction> {
        use horizon_world::native_load_context::LoopSide;
        if context.side() != LoopSide::Server {
            return None;
        }
        let mut reaction = (None, false);
        let mut cancel = false;
        let helmet = crate::crafting::mod_kinds::to_api(self.item)?;
        native_player_gliding_impact(
            context,
            action,
            self.actor,
            &helmet,
            self.persisted,
            &mut reaction,
            &mut cancel,
        );
        cancel.then(|| LandingReaction::Helmet {
            sound: registered_sound(context, reaction.0.as_deref()),
            wear: reaction.1,
        })
    }
}

/// A living entity is about to handle a fall (`LivingFallEvent`). Mods may
/// change the distance and damage multiplier, or cancel fall handling.
#[inline(never)]
pub fn native_living_fall(
    context: &LoopContext,
    entity: u32,
    kind: &str,
    distance: &mut f64,
    multiplier: &mut f32,
    cancel: &mut bool,
) {
    crate::native_client_hooks::living_fall(context, entity, kind, distance, multiplier, cancel);
}

pub fn load(root: &Path, properties: &MovementProperties<'_>) -> Result<BlockLandingRegistry> {
    let raw: BTreeMap<String, serde_json::Value> =
        serde_json::from_slice(&fs::read(root.join("landing-definitions.json"))?)?;
    let mut definitions = BTreeMap::new();
    for (name, d) in raw {
        let float = |key: &str| -> Result<f32> {
            Ok(f32::from_bits(u32::from_str_radix(
                d[key].as_str().ok_or("missing landing float")?,
                16,
            )?))
        };
        let fall = match d["fall"].as_str().ok_or("missing landing family")? {
            "Block" => FallDefinition::Default,
            "HayBlock" => FallDefinition::Hay,
            "HoneyBlock" => FallDefinition::Honey {
                volume: float("volume")?,
                pitch: float("pitch")?,
            },
            "SlimeBlock" => FallDefinition::Slime,
            "PowderSnowBlock" => FallDefinition::PowderSnow,
            "PointedDripstoneBlock" => FallDefinition::PointedDripstone,
            "FarmlandBlock" => FallDefinition::Farmland {
                base_block: d["base"].as_str().ok_or("missing farmland base")?.into(),
            },
            "TurtleEggBlock" => FallDefinition::TurtleEgg,
            _ => return Err("unsupported landing family".into()),
        };
        definitions.insert(
            name,
            LandingDefinition {
                fall,
                shelf_mushroom_bounce: d["bounce"].as_bool().ok_or("missing bounce flag")?,
            },
        );
    }
    Ok(BlockLandingRegistry::new(properties, &definitions)?)
}
#[derive(Clone, Copy, Debug)]
pub enum SceneSound {
    Block(LandingSound),
    SmallFall,
    BigFall,
}
impl SceneSound {
    /// The sound event of `LivingEntity.getFallDamageSound` for a player, if
    /// this is one of the two fall sounds.
    pub fn player_fall_sound(self) -> Option<&'static str> {
        match self {
            Self::BigFall => Some("minecraft:entity.player.big_fall"),
            Self::SmallFall => Some("minecraft:entity.player.small_fall"),
            Self::Block(_) => None,
        }
    }
    /// The sound event a block plays where it is (`Level.playSound(null, pos,
    /// ..)`, which every player hears), if this is one: a turtle egg breaking
    /// (`TurtleEggBlock.decreaseEggs`), a shelf mushroom bouncing.
    pub fn block_sound(self) -> Option<&'static str> {
        match self {
            Self::Block(LandingSound::TurtleEggBreak) => Some("minecraft:entity.turtle.egg_break"),
            Self::Block(LandingSound::ShelfMushroomBounce) => {
                Some("minecraft:block.shelf_mushroom.bounce")
            }
            _ => None,
        }
    }
    /// `HoneyBlock.fallOn`: `entity.playSound` of the slide, and of the block's
    /// own fall sound when the landing still hurts. A player's `playSound`
    /// skips that player; their client plays the same event itself.
    pub fn honey_landing_sound(self) -> Option<&'static str> {
        match self {
            Self::Block(LandingSound::HoneySlide) => Some("minecraft:block.honey_block.slide"),
            Self::Block(LandingSound::HoneyFall) => Some("minecraft:block.honey_block.fall"),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::SmallFall => "small",
            Self::BigFall => "big",
            Self::Block(v) => match v {
                LandingSound::HoneySlide => "honey-slide",
                LandingSound::HoneyFall => "honey-fall",
                LandingSound::TurtleEggBreak => "egg-break",
                LandingSound::ShelfMushroomBounce => "mushroom",
            },
        }
    }
}
/// `HoneyBlock.showParticles`: a slide (entity event 53) sheds 5 honey-block
/// crumbs, a landing (entity event 54) sheds 10.
pub fn honey_particle_count(event: i8) -> Option<u32> {
    match event {
        53 => Some(5),
        54 => Some(10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FallEvent {
    Particles {
        state: BlockStateId,
        position: Vec3,
        count: i32,
        speed: f64,
    },
    HitGround {
        position: Vec3,
        state: BlockStateId,
    },
    Statistic(i32),
    /// `Player.causeFallDamage` returned early for a player who may fly: the
    /// fall's distance and damage multiplier (`PlayerFlyableFallEvent`).
    FlyableFall {
        distance: f64,
        multiplier: f32,
    },
    Damage {
        source: LandingDamage,
        amount: f32,
    },
    Sound {
        sound: SceneSound,
        position: Option<BlockPos>,
        volume: f32,
        pitch: f32,
    },
    BlockFallSound {
        position: BlockPos,
        state: BlockStateId,
    },
    FootwearSound(&'static str),
    HelmetSound {
        name: &'static str,
        eye: [f64; 3],
    },
    HelmetWorn,
    EquipmentEffect {
        player: u128,
        reaction: u128,
        origin: [f64; 3],
        bounds: [[f64; 3]; 2],
        eye: [f64; 3],
        damage: f32,
        sound: Option<(&'static str, f32)>,
    },
    Broadcast(u8),
    Write {
        position: BlockPos,
        /// The block it replaced, so a mod that cancels the change
        /// (`native_entity_change_block_pre`) can have it put back.
        from: BlockStateId,
        state: BlockStateId,
        flags: u32,
    },
    Destroy {
        position: BlockPos,
        drops: bool,
        /// The block it destroyed, for `Level.destroyBlock`'s level event 2001.
        state: BlockStateId,
    },
    BlockEvent {
        kind: LandingEvent,
        position: BlockPos,
        state: BlockStateId,
        with_entity: bool,
    },
    LevelEvent {
        event: i32,
        position: BlockPos,
        data: i32,
    },
}

/// Consume the one equipment effect emitted by this call into the landing code.
pub fn take_fall_reaction(events: &mut Vec<MotionEvent>, start: usize) -> Option<MotionEvent> {
    let at = events
        .get(start..)?
        .iter()
        .position(|event| matches!(event, MotionEvent::Fall(FallEvent::EquipmentEffect { .. })))?;
    Some(events.remove(start + at))
}

impl FallEvent {
    /// Exact-bit diagnostic representation shared by the headless scene probe.
    pub fn trace(self) -> String {
        fn p(v: BlockPos) -> String {
            format!("{},{},{}", v.x, v.y, v.z)
        }
        fn v(v: Vec3) -> String {
            format!(
                "{:016x},{:016x},{:016x}",
                v.x.to_bits(),
                v.y.to_bits(),
                v.z.to_bits()
            )
        }
        match self {
            Self::Particles {
                state,
                position,
                count,
                speed,
            } => format!(
                "particles {} {} {} {:016x}",
                state.0,
                v(position),
                count,
                speed.to_bits()
            ),
            Self::HitGround { position, state } => format!("hit {} {}", v(position), state.0),
            Self::Statistic(n) => format!("stat {n}"),
            Self::FlyableFall {
                distance,
                multiplier,
            } => format!(
                "flyable {:016x} {:08x}",
                distance.to_bits(),
                multiplier.to_bits()
            ),
            Self::Damage { source, amount } => format!(
                "damage {} {:08x}",
                if source == LandingDamage::Fall {
                    "fall"
                } else {
                    "stalagmite"
                },
                amount.to_bits()
            ),
            Self::Sound {
                sound,
                position,
                volume,
                pitch,
            } => format!(
                "sound {} {} {:08x} {:08x}",
                sound.name(),
                position.map_or_else(|| "entity".into(), p),
                volume.to_bits(),
                pitch.to_bits()
            ),
            Self::BlockFallSound { .. } => "block-fall-sound".into(),
            Self::FootwearSound(name) => format!("footwear-sound {name}"),
            Self::HelmetSound { name, .. } => format!("helmet-sound {name}"),
            Self::HelmetWorn => "helmet-worn".into(),
            Self::EquipmentEffect { .. } => "equipment-fall-reaction".into(),
            Self::Broadcast(event) => format!("broadcast {event}"),
            Self::Write {
                position,
                state,
                flags,
                ..
            } => format!("write {} {} {}", p(position), state.0, flags),
            Self::Destroy {
                position, drops, ..
            } => format!("destroy {} {}", p(position), drops),
            Self::BlockEvent {
                kind,
                position,
                state,
                with_entity,
            } => format!(
                "event {} {} {} {}",
                if kind == LandingEvent::BlockChange {
                    "change"
                } else {
                    "destroy"
                },
                p(position),
                state.0,
                with_entity
            ),
            Self::LevelEvent {
                event,
                position,
                data,
            } => format!("level-event {} {} {}", event, p(position), data),
        }
    }
}
impl MotionWorld<'_, '_, '_> {
    fn mod_block_fall(
        &mut self,
        state: BlockStateId,
        position: BlockPos,
        distance: f64,
        damage: f32,
    ) -> f32 {
        crate::block_contact::fall(
            &mut crate::health::DamageLevel::of(
                &mut *self.scene,
                self.assets.registry,
                self.tag_items,
                self.block_registries,
                self.damage_hooks,
            ),
            crate::block_contact::Contact {
                entity: self.player.entity_id,
                kind: "minecraft:player",
                state,
                position,
            },
            distance,
            damage,
        )
    }
    fn fall_event(&mut self, event: FallEvent) {
        self.player.events.push(MotionEvent::Fall(event));
    }
    fn player_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: LandingDamage,
        contact: Option<(BlockStateId, BlockPos, f64)>,
    ) -> Result<bool> {
        // Player.causeFallDamage exempts players with the mayfly ability,
        // including creative players who are currently walking.
        if self.player.may_fly {
            if let Some((state, position, original_distance)) = contact {
                self.mod_block_fall(state, position, original_distance, 0.0);
            }
            // Only for mods' `player_flyable_fall`: without them the landing
            // stays as vanilla traces it.
            if !self.player.replaying && crate::native_client_hooks::has_mods() {
                self.fall_event(FallEvent::FlyableFall {
                    distance,
                    multiplier,
                });
            }
            return Ok(false);
        }
        // Record centimeters before delegating to LivingEntity.
        if distance >= 2.0 || distance.is_nan() {
            let scaled = distance * 100.0;
            let floor = scaled.floor();
            let rounded = (if scaled - floor >= 0.5 {
                floor + 1.0
            } else {
                floor
            }) as i64;
            self.fall_event(FallEvent::Statistic(rounded as i32));
        }
        let mut distance = distance;
        let mut multiplier = multiplier;
        let mut cancel = false;
        if !self.player.replaying {
            native_living_fall(
                self.loop_context,
                self.player.entity_id as u32,
                "minecraft:player",
                &mut distance,
                &mut multiplier,
                &mut cancel,
            );
        }
        // A canceled fall still lands on the block (`Block.fallOn`).
        if cancel {
            if let Some((state, position, original_distance)) = contact {
                self.mod_block_fall(state, position, original_distance, 0.0);
            }
            return Ok(false);
        }
        living_fall::cause_living_fall_damage(
            &mut FallCall {
                world: self,
                source,
                contact,
            },
            distance,
            multiplier,
        )
    }
    /// `Entity.propagateFallToPassengers`: the call on this player, whose
    /// vehicle fell `distance` blocks at `multiplier`, so the player takes
    /// `Player.causeFallDamage` of it.
    pub fn cause_mount_fall(&mut self, distance: f64, multiplier: f32) -> Result<bool> {
        BlockLandingWorld::cause_fall_damage(self, distance, multiplier, LandingDamage::Fall)
    }
}
impl FallDamageWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type State = BlockStateId;
    fn is_in_water(&mut self) -> Result<bool> {
        Ok(self.player.fluids.was_touching_water)
    }
    fn fall_distance(&self) -> f64 {
        self.player.fall_distance
    }
    fn set_fall_distance(&mut self, value: f64) {
        self.player.fall_distance = value;
    }
    fn fall_on(&mut self, state: BlockStateId, position: BlockPos, distance: f64) -> Result<()> {
        let previous = self.player.block_landing.take();
        if crate::block_contact::fall_listens(state) {
            self.player.block_landing = Some((state, position, distance));
        }
        // Keep template landing rules (hay, honey, slime, etc.) in control.
        let result = self
            .resources
            .landing
            .fall_on(self, state, position, LandingEntityKind::Player, distance)
            .map_err(|e| e.to_string().into());
        let pending = self.player.block_landing.take();
        self.player.block_landing = previous;
        if result.is_ok()
            && let Some((state, position, distance)) = pending
        {
            // Some landing rules never call causeFallDamage.
            self.mod_block_fall(state, position, distance, 0.0);
        }
        result
    }

    fn position(&self) -> Vec3 {
        self.player.position
    }
    fn supporting_block(&self) -> Option<BlockPos> {
        self.player.ground.supporting_block
    }
    fn block_state(&mut self, p: BlockPos) -> Result<BlockStateId> {
        Ok(self.scene.block(p.x, p.y, p.z))
    }
    fn hit_ground(&mut self, position: Vec3, state: BlockStateId) -> Result<()> {
        self.fall_event(FallEvent::HitGround { position, state });
        Ok(())
    }
    fn reset_fall_distance(&mut self) -> Result<()> {
        self.player.fall_distance = 0.0;
        Ok(())
    }
}
impl LivingLandingWorld for MotionWorld<'_, '_, '_> {
    fn update_fluid_interaction(&mut self) -> Result<bool> {
        self.update_fluids()
    }
    fn is_server_level(&self) -> bool {
        true
    }
    fn on_changed_block(&mut self, _: BlockPos) -> Result<()> {
        Ok(())
    } // empty equipment
    fn safe_fall_distance(&mut self) -> Result<f64> {
        self.player.attribute("minecraft:safe_fall_distance")
    }
    fn state_is_air(&mut self, state: BlockStateId) -> Result<bool> {
        Ok(self.scene.is_air(state))
    }
    fn block_position(&mut self) -> Result<BlockPos> {
        Ok(BlockPos::containing(self.player.position))
    }
    fn landing_particles(
        &mut self,
        state: BlockStateId,
        position: Vec3,
        count: i32,
        speed: f64,
    ) -> Result<()> {
        self.fall_event(FallEvent::Particles {
            state,
            position,
            count,
            speed,
        });
        Ok(())
    }
    fn clear_last_climbable_position(&mut self) {
        self.player.last_climbable = None;
    }
}
impl BlockPushWorld for MotionWorld<'_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    fn collision_shape(&mut self, state: BlockStateId, p: BlockPos) -> Result<VoxelShape> {
        let mut world = SceneCollisions {
            world: self.scene,
            binding: self.collision_binding,
            subject: None,
        };
        self.collision_binding
            .collision_shape(&mut world, state, [p.x, p.y, p.z])
            .map_err(|e| e.to_string().into())
    }
    fn entities(&mut self, bounds: Aabb) -> Result<Vec<EntityIdentity>> {
        Ok(if bounds.intersects(self.player.bounds()) {
            vec![EntityIdentity(0)]
        } else {
            vec![]
        })
    }
    fn entity_bounds(&self, _: EntityIdentity) -> Aabb {
        self.player.bounds()
    }
    fn teleport_relative(&mut self, _: EntityIdentity, movement: Vec3) -> Result<()> {
        self.player.position = Vec3::new(
            self.player.position.x + movement.x,
            self.player.position.y + movement.y,
            self.player.position.z + movement.z,
        );
        Ok(())
    }
}
impl BlockLandingWorld for MotionWorld<'_, '_, '_> {
    type DamageSource = LandingDamage;
    type Sound = SceneSound;
    fn is_server_level(&self) -> bool {
        // A remote client's world is a client level: it never tramples,
        // the server's block change and teleport do it (FarmlandBlock.fallOn
        // and TurtleEggBlock.fallOn check `level instanceof ServerLevel`).
        !self.player.client_side
    }
    fn is_client_side(&mut self) -> Result<bool> {
        Ok(self.player.client_side)
    }
    fn random_float(&mut self) -> Result<f32> {
        Ok(self.scene.random.next_float())
    }
    fn random_int(&mut self, bound: i32) -> Result<i32> {
        Ok(self.scene.random.next_int_bound(bound)?)
    }
    fn under_spawn_protection(&mut self, _: BlockPos) -> Result<bool> {
        Ok(false)
    }
    fn mob_griefing(&mut self) -> Result<bool> {
        Ok(true)
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        (self.player.dimensions.width, self.player.dimensions.height)
    }
    fn cancel_farmland_trample(
        &mut self,
        position: BlockPos,
        state: BlockStateId,
        distance: f64,
    ) -> bool {
        let mut cancel = false;
        native_farmland_trample(
            self.loop_context,
            self.player.entity_id as u32,
            [position.x, position.y, position.z],
            state.0,
            distance,
            &mut cancel,
        );
        cancel
            || self
                .landing_equipment
                .as_ref()
                .and_then(|equipment| equipment.boots)
                .is_some_and(|footwear| {
                    footwear
                        .reaction(self.loop_context, "farmland_trample")
                        .is_some()
                })
    }
    fn suppressing_bounce(&mut self) -> Result<bool> {
        Ok(self.player.shift_key_down)
    }
    fn damage_source(
        &mut self,
        _: DamageSourceOwner,
        kind: LandingDamage,
    ) -> Result<LandingDamage> {
        Ok(kind)
    }
    fn cause_fall_damage(
        &mut self,
        distance: f64,
        multiplier: f32,
        source: LandingDamage,
    ) -> Result<bool> {
        let contact = self.player.block_landing.take();
        self.player_fall_damage(distance, multiplier, source, contact)
    }
    fn known_sound(&self, s: LandingSound) -> SceneSound {
        SceneSound::Block(s)
    }
    fn living_fall_sound(&mut self, big: bool) -> Result<SceneSound> {
        Ok(if big {
            SceneSound::BigFall
        } else {
            SceneSound::SmallFall
        })
    }
    fn entity_sound(&mut self, sound: SceneSound, volume: f32, pitch: f32) -> Result<()> {
        self.fall_event(FallEvent::Sound {
            sound,
            position: None,
            volume,
            pitch,
        });
        Ok(())
    }
    fn block_sound(
        &mut self,
        position: BlockPos,
        sound: SceneSound,
        volume: f32,
        pitch: f32,
    ) -> Result<()> {
        self.fall_event(FallEvent::Sound {
            sound,
            position: Some(position),
            volume,
            pitch,
        });
        Ok(())
    }
    fn broadcast_entity_event(&mut self, event: u8) -> Result<()> {
        self.fall_event(FallEvent::Broadcast(event));
        Ok(())
    }
    fn destroy_block(&mut self, position: BlockPos, drops: bool) -> Result<bool> {
        let air = self
            .assets
            .registry
            .default_state("minecraft:air")
            .ok_or("missing air")?;
        let state = self.scene.block(position.x, position.y, position.z);
        self.scene.set(position.x, position.y, position.z, air)?;
        self.fall_event(FallEvent::Destroy {
            position,
            drops,
            state,
        });
        Ok(true)
    }
    fn set_block(&mut self, position: BlockPos, state: BlockStateId, flags: u32) -> Result<bool> {
        let from = self.scene.block(position.x, position.y, position.z);
        self.scene.set(position.x, position.y, position.z, state)?;
        self.fall_event(FallEvent::Write {
            position,
            from,
            state,
            flags,
        });
        Ok(true)
    }
    fn block_event(
        &mut self,
        kind: LandingEvent,
        position: BlockPos,
        state: BlockStateId,
        with_entity: bool,
    ) -> Result<()> {
        self.fall_event(FallEvent::BlockEvent {
            kind,
            position,
            state,
            with_entity,
        });
        Ok(())
    }
    fn level_event(&mut self, event: i32, position: BlockPos, data: i32) -> Result<()> {
        self.fall_event(FallEvent::LevelEvent {
            event,
            position,
            data,
        });
        Ok(())
    }
}
struct FallCall<'a, 'p, 'w, 'r> {
    world: &'a mut MotionWorld<'p, 'w, 'r>,
    source: LandingDamage,
    contact: Option<(BlockStateId, BlockPos, f64)>,
}
impl LivingFallWorld for FallCall<'_, '_, '_, '_> {
    type Error = Box<dyn std::error::Error>;
    type Sound = SceneSound;
    fn impact_position(&self) -> Option<Vec3> {
        self.world.player.impulse.impact_position
    }
    fn y(&self) -> f64 {
        self.world.player.position.y
    }
    fn reset_impulse(&mut self) -> Result<()> {
        self.world.player.impulse.reset();
        Ok(())
    }
    fn try_reset_impulse(&mut self) -> Result<()> {
        self.world.player.impulse.try_reset();
        Ok(())
    }
    fn immune_to_fall_damage(&mut self) -> Result<bool> {
        Ok(false)
    } // player type
    fn propagate_fall_to_passengers(&mut self, _: f64, _: f32) -> Result<()> {
        Ok(())
    } // no passengers
    fn safe_fall_distance(&mut self) -> Result<f64> {
        self.world.player.attribute("minecraft:safe_fall_distance")
    }
    fn fall_damage_multiplier(&mut self) -> Result<f64> {
        self.world
            .player
            .attribute("minecraft:fall_damage_multiplier")
    }
    fn modify_fall_damage(&mut self, _distance: f64, damage: f32) -> Result<f32> {
        Ok(self.contact.map_or(damage, |(state, position, distance)| {
            self.world.mod_block_fall(state, position, distance, damage)
        }))
    }
    fn fall_sound(&mut self, big: bool) -> Result<SceneSound> {
        self.world.living_fall_sound(big)
    }
    fn play_fall_sound(&mut self, sound: SceneSound) -> Result<()> {
        self.world.entity_sound(sound, 1.0, 1.0)
    }
    /// `LivingEntity.playBlockFallSound`. The server keeps no Silent flag for a player,
    /// so its `isSilent()` return is the landing client's (`remote::fall_sound`).
    fn play_block_fall_sound(&mut self) -> Result<()> {
        let p = self.world.player.position;
        let position = BlockPos::containing(Vec3::new(p.x, p.y - 0.20000000298023224, p.z));
        let state = self.world.scene.block(position.x, position.y, position.z);
        if !self.world.scene.is_air(state) {
            self.world
                .fall_event(FallEvent::BlockFallSound { position, state });
        }
        Ok(())
    }
    fn hurt(&mut self, amount: f32) -> Result<()> {
        self.world.fall_event(FallEvent::Damage {
            source: self.source,
            amount,
        });
        let source = self.world.health.landing_source(self.source);
        let action = matches!(self.source, LandingDamage::Fall).then_some("fall_damage");
        self.world.impact_damage(source, amount, action)
    }
}

impl MotionWorld<'_, '_, '_> {
    /// The server's fall or gliding wall-hit damage boundary, before armor or health changes.
    pub(crate) fn impact_damage(
        &mut self,
        source: horizon_world::damage_source::DamageTypeId,
        amount: f32,
        action: Option<&'static str>,
    ) -> Result<()> {
        let (boots, helmet) = action
            .and_then(|action| self.landing_equipment.as_ref().map(|gear| (action, gear)))
            .map_or((None, None), |(action, gear)| {
                let boots = (action == "fall_damage").then_some(gear.boots).flatten();
                (boots, gear.helmet_at_impact(self.player.fall_flying))
            });
        let reaction = std::cell::Cell::new(LandingReaction::None);
        let rules = self.scene.health_rules;
        let pause_at = self.landing_equipment.as_ref().and_then(|gear| {
            (action == Some("fall_damage") && gear.check_environment_damage)
                .then_some(gear.origin.unwrap_or(self.player.position))
        });
        let mut level = crate::health::DamageLevel::of(
            self.scene,
            self.assets.registry,
            self.tag_items,
            self.block_registries,
            self.damage_hooks,
        )
        .with_landing_impact(
            boots,
            helmet,
            self.loop_context,
            action.unwrap_or(""),
            &reaction,
        );
        let paused_fall = if let Some(position) = pause_at {
            let mut hit = self.player.start_hurt(
                &mut level,
                self.health,
                rules,
                source,
                amount,
                None,
                false,
                0,
                false,
                false,
                true,
                None,
                false,
            )?;
            match self
                .player
                .admitted_reaction_damage(&mut level, self.health, rules, &mut hit)?
            {
                Some(damage) if level.fall_equipment_cancels(damage) => None,
                Some(damage) if damage > 0.0 => Some(PausedFallDamage {
                    damage,
                    position,
                    event_index: self.player.events.len(),
                    hit,
                }),
                _ => {
                    self.player
                        .finish_hurt(&mut level, self.health, rules, hit)?;
                    None
                }
            }
        } else {
            self.player
                .hurt(&mut level, self.health, rules, source, amount)?;
            None
        };
        if let Some(fall) = paused_fall
            && let Some(gear) = self.landing_equipment.as_mut()
        {
            gear.paused_fall = Some(Box::new(fall));
        }
        match reaction.get() {
            LandingReaction::Footwear(Some(sound)) => {
                self.fall_event(FallEvent::FootwearSound(sound))
            }
            LandingReaction::Helmet { sound, wear } => {
                if let Some(name) = sound {
                    self.helmet_sound(name);
                }
                if wear {
                    self.wear_impact_helmet();
                }
            }
            effect @ LandingReaction::Effect { .. } => {
                let at = self
                    .landing_equipment
                    .as_ref()
                    .and_then(|gear| gear.origin)
                    .unwrap_or(self.player.position);
                self.player
                    .events
                    .extend(effect.effect_at(at, self.player.dimensions));
            }
            _ => {}
        }
        Ok(())
    }

    fn helmet_sound(&mut self, name: &'static str) {
        let at = self.player.position;
        let eye = [
            at.x,
            at.y + f64::from(self.player.dimensions.eye_height),
            at.z,
        ];
        self.fall_event(FallEvent::HelmetSound { name, eye });
    }

    fn wear_impact_helmet(&mut self) {
        let Some(gear) = self
            .landing_equipment
            .as_mut()
            .and_then(|gear| gear.helmet.as_mut())
        else {
            return;
        };
        let Some(helmet) = gear.slot.as_mut().filter(|helmet| helmet.count != 0) else {
            return;
        };
        let info = self.tag_items.and_then(|items| items.get(&helmet.item));
        let max = info.map_or(0, |info| info.max_damage) as i32;
        if gear.actor.1 == GameMode::Creative
            || max <= 0
            || helmet.carries_component(info, "minecraft:unbreakable")
        {
            return;
        }
        let level = helmet.tag_enchantment_level("minecraft:unbreaking");
        if level > 0 && self.player.random.next_double() >= 0.6 + 0.4 / (f64::from(level) + 1.0) {
            return;
        }
        let damage = helmet.effective_damage(info);
        if damage >= max {
            // ItemMeta wear consumes the item on the impact after reaching material max damage.
            *gear.slot = None;
            self.helmet_sound("minecraft:entity.item.break");
        } else {
            helmet.damage = (damage + 1) as u32;
        }
        self.fall_event(FallEvent::HelmetWorn);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// Both landing sources use the world's frozen definitions and current
    /// damage tags, including a tag-only reload with no pack definitions left.
    #[test]
    #[ignore = "daily"]
    fn datapack_damage_landing_uses_world_armor_tags_and_exhaustion() {
        use crate::{assets::Assets, motion::MotionResources, world::World};
        use horizon_world::loot::PackOverlay;
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        let assets = Assets::load(&root).unwrap();
        let render = assets
            .reload(&horizon_assets::resource_source::ResourceRoot::directory(
                root.to_path_buf(),
            ))
            .unwrap();
        let collision = crate::collisions::load(&root, assets.registry).unwrap();
        let resources = MotionResources::load(&root, &assets).unwrap();
        let files = vec![
            (
                "damage_type/minecraft:fall".into(),
                vec![br#"{"message_id":"fall","scaling":"never","exhaustion":0.75}"#.to_vec()],
            ),
            (
                "damage_type/minecraft:stalagmite".into(),
                vec![br#"{"message_id":"stalagmite","scaling":"never","exhaustion":0.5}"#.to_vec()],
            ),
            (
                "tags/minecraft:damage_type/bypasses_armor".into(),
                vec![br#"{"replace":true,"values":[]}"#.to_vec()],
            ),
        ];
        let open = resources
            .health
            .with_packs(&root, &PackOverlay::new(&files), false)
            .unwrap();
        let reloaded = open.with_packs(&root, &PackOverlay::NONE, true).unwrap();
        let mut scene = World::test_scene(&assets, &render, 1).unwrap();
        for (source, exhaustion, name) in [
            (LandingDamage::Fall, 0.75, "minecraft:fall"),
            (LandingDamage::Stalagmite, 0.5, "minecraft:stalagmite"),
        ] {
            for (health, bypass) in [(&resources.health, true), (&open, false), (&reloaded, true)] {
                let mut player = crate::motion::PlayerMotion::new(&resources, Vec3::ZERO).unwrap();
                let key = player.attributes.key("minecraft:armor").unwrap();
                let armor = player.attributes.get_instance(Some(key)).unwrap().unwrap();
                player
                    .attributes
                    .edit(&armor, |a, o| a.set_base_value(20.0, o))
                    .unwrap();
                BlockLandingWorld::cause_fall_damage(
                    &mut MotionWorld {
                        landing_equipment: None,
                        entities: &[],
                        player: &mut player,
                        scene: &mut scene,
                        assets: &assets,
                        collision_binding: &collision,
                        resources: &resources,
                        health,
                        tag_items: None,
                        block_registries: None,
                        loop_context: crate::test_resources::server_loop_context(),
                        damage_hooks: &crate::health::DamageHooks::NONE,
                    },
                    7.0,
                    1.0,
                    source,
                )
                .unwrap();
                if bypass {
                    assert_eq!(player.health.health(), 16.0, "{name}");
                } else {
                    assert!(
                        player.health.health() > 16.0 && player.health.health() < 20.0,
                        "{name}"
                    );
                }
                assert_eq!(player.hurt_log.last().unwrap().source, name);
                let expected = if std::ptr::eq(health, &resources.health) {
                    0.0
                } else {
                    exhaustion
                };
                assert_eq!(player.health.food.exhaustion, expected, "{name}");
            }
        }
    }

    #[test]
    fn fall_sounds_map_to_player_events() {
        assert_eq!(
            SceneSound::BigFall.player_fall_sound(),
            Some("minecraft:entity.player.big_fall")
        );
        assert_eq!(
            SceneSound::SmallFall.player_fall_sound(),
            Some("minecraft:entity.player.small_fall")
        );
        assert_eq!(
            SceneSound::Block(LandingSound::HoneyFall).player_fall_sound(),
            None
        );
    }

    /// The sounds a block plays where it is; the honey block's are the
    /// entity's own (`Entity.playSound`) and the fall sounds the player's.
    #[test]
    fn block_sounds_map_to_block_events() {
        assert_eq!(
            SceneSound::Block(LandingSound::TurtleEggBreak).block_sound(),
            Some("minecraft:entity.turtle.egg_break")
        );
        assert_eq!(
            SceneSound::Block(LandingSound::ShelfMushroomBounce).block_sound(),
            Some("minecraft:block.shelf_mushroom.bounce")
        );
        assert_eq!(
            SceneSound::Block(LandingSound::HoneySlide).block_sound(),
            None
        );
        assert_eq!(SceneSound::BigFall.block_sound(), None);
    }

    /// `HoneyBlock.fallOn` plays the slide, then the block's fall sound when
    /// the landing hurts. `showSlideParticles` is entity event 53 (5 crumbs);
    /// `showJumpParticles` is entity event 54 (10).
    #[test]
    fn a_honey_landing_names_its_sounds_and_particle_counts() {
        assert_eq!(
            SceneSound::Block(LandingSound::HoneySlide).honey_landing_sound(),
            Some("minecraft:block.honey_block.slide")
        );
        assert_eq!(
            SceneSound::Block(LandingSound::HoneyFall).honey_landing_sound(),
            Some("minecraft:block.honey_block.fall")
        );
        assert_eq!(SceneSound::SmallFall.honey_landing_sound(), None);
        assert_eq!(honey_particle_count(53), Some(5));
        assert_eq!(honey_particle_count(54), Some(10));
        assert_eq!(honey_particle_count(46), None);
    }
}
