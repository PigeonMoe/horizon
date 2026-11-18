use super::*;
use mod_api::{
    BlockEntityRef, BlockPos, Compound, DimensionKey, Flow, LevelDimension, LevelHost, LevelRef,
    PistonPreEvent, Tag, WorldEpoch,
};

const SOURCE: [i32; 3] = [8, 64, 8];

struct World(BlockEntityRef);

impl LevelHost for World {
    fn block_entity_at(&self, pos: [i32; 3]) -> Option<BlockEntityRef> {
        (pos == SOURCE).then_some(self.0)
    }

    fn block_entity_saved_attachment(
        &self,
        entity: BlockEntityRef,
        namespace: &str,
        name: &str,
    ) -> Option<Compound> {
        (entity == self.0 && namespace == "test" && name == "machine")
            .then(|| Compound::from([("protected".into(), Tag::Byte(1))]))
    }
}

pub(super) struct PistonPreHooks;

impl PistonPreHooks {
    pub(super) fn piston_pre(
        &self,
        level: &dyn mod_api::LevelHost,
        event: &mut mod_api::PistonPreEvent<'_>,
    ) -> mod_api::Flow {
        let BlockPos { x, y, z } = event.piston;
        let marker = level
            .block_entity_at([x, y, z])
            .and_then(|entity| level.block_entity_saved_attachment(entity, "test", "machine"));
        if marker.as_ref().and_then(|tag| tag.get(&"protected".into())) == Some(&Tag::Byte(1)) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

#[test]
fn wire_loopback_piston_checks_a_resident_block_entity_marker() {
    mod_api::__private::initialize().unwrap();
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
    install_counter();
    let world = World(mod_api::__private::game::block_entity_ref(
        LevelRef::new(WorldEpoch::new(1), DimensionKey::OVERWORLD),
        SOURCE,
        1,
    ));
    let module = WasmMod {
        subscribed: &[],
        listeners: &[],
        group: 0,
        corrupt: false,
    };
    for (x, protected) in [(8, true), (9, false)] {
        let mut event = PistonPreEvent {
            dimension: LevelDimension::Overworld,
            piston: BlockPos::new(x, 64, 8),
            direction: [-1, 0, 0],
            extending: false,
            push: &[],
            destroy: &[],
            cancelled: false,
        };
        assert_eq!(
            matches!(module.piston_pre(&world, &mut event), Flow::Cancel),
            protected
        );
    }
}
