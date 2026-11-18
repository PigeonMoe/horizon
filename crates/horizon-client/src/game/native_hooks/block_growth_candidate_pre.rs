use super::*;

impl GameLevel<'_> {
    pub(in crate::game) fn native_block_growth_candidate_pre(
        &self,
        call: mod_api::__private::BlockGrowthCandidatePreCall<'_>,
    ) {
        self.fire_block_growth_candidate_pre(call);
    }
}
