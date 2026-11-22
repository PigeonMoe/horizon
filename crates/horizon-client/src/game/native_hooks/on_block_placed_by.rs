use super::*;
use horizon_world::level::PlacementPreview;

fn block_face(direction: Direction) -> mod_api::BlockFace {
    match direction {
        Direction::Down => mod_api::BlockFace::Down,
        Direction::Up => mod_api::BlockFace::Up,
        Direction::North => mod_api::BlockFace::North,
        Direction::South => mod_api::BlockFace::South,
        Direction::West => mod_api::BlockFace::West,
        Direction::East => mod_api::BlockFace::East,
    }
}

impl Game<'_> {
    /// A player's item can refuse the resolved placement before it writes.
    pub(in crate::game) fn native_block_item_place_pre(
        &mut self,
        call: super::fire::BlockItemPlacePreCall<'_>,
    ) {
        self.fire_block_item_place_pre(call);
    }

    pub(in crate::game) fn block_item_place_refused(
        &mut self,
        stack: &ItemStack,
        preview: PlacementPreview,
        ctx: &PlaceContext,
    ) -> bool {
        let Some(player) = self.native_acting_player() else {
            return false;
        };
        let BlockPos { x, y, z } = preview.pos;
        let BlockPos {
            x: cx,
            y: cy,
            z: cz,
        } = ctx.clicked_pos;
        let mut canceled = false;
        self.native_block_item_place_pre(super::fire::BlockItemPlacePreCall {
            stack,
            player,
            pos: [x, y, z],
            clicked_pos: [cx, cy, cz],
            state: preview.state,
            face: block_face(ctx.face),
            canceled: &mut canceled,
        });
        canceled
    }

    /// `BlockItemExtension.onBlockPlacedBy`: once the block's final state,
    /// stack data and `Block.setPlacedBy` companions exist, before consumption.
    pub(in crate::game) fn native_on_block_placed_by(
        &mut self,
        call: super::fire::OnBlockPlacedByCall<'_>,
    ) {
        self.fire_on_block_placed_by(call);
    }

    /// Dispatch only for this stack's subscribers, without converting an
    /// unobserved stack or constructing a placement event.
    pub(in crate::game) fn block_item_placed(
        &mut self,
        stack: &ItemStack,
        placer: Option<mod_api::PlayerRef>,
        pos: BlockPos,
        ctx: &PlaceContext,
    ) {
        if !mod_api::__private::item_listens(mod_api::HookId::OnBlockPlacedBy, &stack.item) {
            return;
        }
        // setPlacedBy (for example a jukebox) can change the state again.
        let state = self.world.block(pos.x, pos.y, pos.z);
        self.native_on_block_placed_by(super::fire::OnBlockPlacedByCall {
            stack,
            placer,
            pos: [pos.x, pos.y, pos.z],
            state,
            face: block_face(ctx.face),
            click: [ctx.hit.x, ctx.hit.y, ctx.hit.z],
        });
    }
}
