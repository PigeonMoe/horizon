use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// An entity is about to change a block, such as by trampling farmland or
    /// stripping copper patina with lightning. Dispatch lifts the engine's
    /// `from` and `to` to API values. Canceling prevents the block write and its
    /// neighbour updates (@API:entity_change_block_pre@).
    pub(in crate::game) fn native_entity_change_block_pre(
        &mut self,
        call: super::fire::EntityChangeBlockPreCall<'_>,
    ) {
        self.fire_entity_change_block_pre(call);
    }
}
