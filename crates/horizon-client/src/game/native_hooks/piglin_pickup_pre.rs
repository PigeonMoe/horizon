use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_piglin_pickup_pre(
        &self,
        piglin: u32,
        stack: &ItemStack,
        cancel: &mut bool,
    ) {
        self.fire_piglin_pickup_pre(piglin, stack, cancel);
    }
}
