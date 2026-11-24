use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_player_command_literals_sent(
        &mut self,
        player: u128,
        name: &str,
        permission: u8,
        hidden_literals: &mut Vec<String>,
    ) {
        self.fire_player_command_literals_sent(player, name, permission, hidden_literals);
    }
}
