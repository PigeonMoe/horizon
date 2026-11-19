use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_piglin_throw_item(
        &self,
        piglin: u32,
        stack: &ItemStack,
        replacement: &mut Option<ItemStack>,
    ) {
        self.fire_piglin_throw_item(piglin, stack, replacement);
    }
}
