//! Storage that is not a block entity behind a mod menu's slots (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` (a stack held inside another item) and `ResourceHandlerSlot`
//! (an index of an item handler). A mod opens its menu type with `PlayerRef::open_mod_menu`
//! (arismoko/horizon#3384) and then backs its container slots with [`PlayerRef::back_mod_menu_slots`]; a click
//! or shift-click on such a slot, and an [`ItemPlace::MenuSlot`] write to it, go through that
//! storage's own handler in one journalled transaction, as a block entity's slots do.
use crate::{
    BlockEntityRef, BlockKey, Capability, ENTITY_ITEMS, EntityRef, Hand, ITEM_ITEMS, ITEMS,
    ItemPlace, ItemResource, LevelRef, PlaceProvider, PlayerRef, ResourceHandler, ServerHost,
    Transaction,
};
use std::sync::Arc;

/// The storage one container slot of a mod menu opened with no block shows
/// ([`MenuSlotBacking::Place`](crate::MenuSlotBacking::Place)).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MenuSlotPlace {
    /// Slot `slot` of the container inside the stack the menu's player holds in `hand`, through
    /// that stack's [`ITEM_ITEMS`] handler: a backpack's contents (`StackCopySlot`). The menu
    /// closes once the hand holds no such container, and the player's inventory
    /// slot that holds it takes no click while the menu is open, so the stack can't go into
    /// itself. The stack is bound when the slots are backed: the menu closes, before the next
    /// click or at the next change notice, once that inventory slot no longer holds it (another
    /// stack put in its place, the selected hotbar slot changed).
    HeldItem {
        /// The hand holding the container.
        hand: Hand,
        /// The slot's index in the container.
        slot: usize,
    },
    /// Slot `slot` of `entity`'s [`ENTITY_ITEMS`] handler, as a mod's provider answers it for that
    /// entity (`ResourceHandlerSlot`). The menu closes once the entity is gone, found before the
    /// next click or at the next change notice.
    EntityItems {
        /// The entity holding the inventory.
        entity: EntityRef,
        /// The slot's index in the handler.
        slot: usize,
    },
    /// A display or button slot supplied by the mod. It does not accept inventory moves.
    Decoration,
    /// A slot of the current block entity's unsided [`ITEMS`] capability. This binds to a
    /// block entity generation, so replacing it at the same position closes the menu.
    BlockEntityItems {
        /// The block entity holding the inventory.
        entity: BlockEntityRef,
        /// The slot in its item handler.
        slot: usize,
        /// False for slots that the player can extract from but cannot fill, such as outputs.
        allow_insert: bool,
    },
    /// A slot of a block capability without a block entity. The provider must stop resolving
    /// when its inventory is gone. The owning mod invalidates viewers if an identical carrier
    /// and its inventory are both replaced before the menu is checked again.
    BlockItems {
        /// The inventory's level.
        level: LevelRef,
        /// The block providing the inventory.
        pos: [i32; 3],
        /// The carrier block's identity. Properties may change while the menu is open.
        expected_block: BlockKey,
        /// The slot in its item handler.
        slot: usize,
        /// Whether players can place items in the slot through this menu.
        allow_insert: bool,
    },
    /// A slot of a registered mod place provider. Other viewers can show the same storage.
    /// Disabled providers or keys that no longer resolve close the menu.
    ModSlot {
        /// Name passed to `Registrar::place_provider`.
        provider: &'static str,
        /// The provider's own key for this slot.
        key: u64,
    },
}

/// Why [`PlayerRef::back_mod_menu_slots`] backed nothing; the menu and its slots stay as they
/// were.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MenuSlotBackRefusal {
    /// The player is not online here (also a host with no game behind it), or `window` is not
    /// the menu they have open.
    NoPlayerOrWindow,
    /// The open menu is not a mod menu opened with no block (its slots are a block entity's,
    /// an entity's or the game's own), so its slots cannot show another storage.
    UnsupportedMenu,
    /// More places than the menu has container slots.
    TooManyPlaces,
    /// `places[index]` names no storage now (no container in that hand, the entity gone or
    /// without an item handler, a slot past the handler's size).
    PlaceUnresolved {
        /// The place's index in `places`.
        index: usize,
    },
}

impl crate::NoBackend for MenuSlotBackRefusal {
    /// No game behind the host: no player is online there.
    fn no_backend() -> Self {
        Self::NoPlayerOrWindow
    }
}

impl MenuSlotPlace {
    /// The handler this place reads and writes, and the index in it, for the menu of `player`;
    /// None once it no longer resolves. It builds no [`ItemPlace`] on the way.
    pub(crate) fn handler(
        self,
        host: &mut dyn ServerHost,
        player: PlayerRef,
    ) -> Option<(Capability<dyn ResourceHandler<ItemResource>>, usize)> {
        match self {
            Self::HeldItem { hand, slot } => {
                let held = ItemPlace::PlayerHand {
                    player: player.uuid(),
                    hand,
                };
                Some((held.capability(host, &ITEM_ITEMS)?, slot))
            }
            Self::EntityItems { entity, slot } => {
                Some((entity.capability(host, &ENTITY_ITEMS, ())?, slot))
            }
            Self::Decoration => None,
            Self::BlockEntityItems { entity, slot, .. } => {
                let level = entity.level;
                let pos = entity.position;
                if host.block_entity(level, pos) != Some(entity)
                    || player.entity().level(host) != Some(level)
                {
                    return None;
                }
                Some((level.capability(host, &ITEMS, pos, None)?, slot))
            }
            Self::BlockItems {
                level,
                pos,
                expected_block,
                slot,
                ..
            } => {
                if player.entity().level(host) != Some(level)
                    || level.block_state(host, pos)?.block() != expected_block
                {
                    return None;
                }
                Some((level.capability(host, &ITEMS, pos, None)?, slot))
            }
            Self::ModSlot { provider, key } => {
                let (provider, owner) = host.capability_providers().place_provider(provider)?;
                let _scope = crate::mark_mod(owner, "menu_slot_exists");
                if !provider.menu_slot_exists(host, key) {
                    return None;
                }
                Some((
                    Capability {
                        handler: Box::new(ModMenuSlot { provider, key }),
                        owner: Some(owner),
                    },
                    0,
                ))
            }
        }
    }

    /// Whether the menu permits insertion through this backing. The inventory may apply
    /// additional item or capacity filters.
    pub fn allows_insertion(self) -> bool {
        !matches!(
            self,
            Self::Decoration
                | Self::BlockEntityItems {
                    allow_insert: false,
                    ..
                }
                | Self::BlockItems {
                    allow_insert: false,
                    ..
                }
        )
    }

    /// Whether this place still names storage, for the menu of `player`: false closes the menu.
    pub fn resolves(self, host: &mut dyn ServerHost, player: PlayerRef) -> bool {
        self == Self::Decoration
            || self
                .handler(host, player)
                .is_some_and(|(handler, slot)| handler.with_handler(|h| slot < h.size(host)))
    }
}

struct ModMenuSlot {
    provider: Arc<dyn PlaceProvider>,
    key: u64,
}

impl ResourceHandler<ItemResource> for ModMenuSlot {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        1
    }

    fn resource(&self, host: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
        Some(ItemResource::of(&self.provider.stack(host, self.key)?))
    }

    fn amount(&self, host: &mut dyn ServerHost, _: usize) -> u32 {
        self.provider
            .stack(host, self.key)
            .map_or(0, |stack| stack.count())
    }

    fn capacity(&self, host: &mut dyn ServerHost, _: usize, resource: &ItemResource) -> u32 {
        self.provider.menu_slot_capacity(host, self.key, resource)
    }

    fn insert(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        _: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.provider.insert(host, tx, self.key, resource, amount)
    }

    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        _: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        self.provider.extract(host, tx, self.key, resource, amount)
    }
}
