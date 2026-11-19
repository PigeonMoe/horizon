use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_recipe_result(
        &self,
        call: mod_api::__private::PlayerRecipeResultCall<'_>,
    ) {
        self.fire_player_recipe_result(call);
    }
}
