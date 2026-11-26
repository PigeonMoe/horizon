use super::*;
use horizon_client_core::suggestion::Suggestion;

fn hide_literal(packet: &mut play::Commands, path: &str) {
    let mut literals = path.split_ascii_whitespace().peekable();
    let mut parent = packet.root as usize;
    while let Some(literal) = literals.next() {
        let Some(node) = packet.nodes.get(parent) else {
            return;
        };
        let Some(child) = node.children.iter().copied().find(|child| {
            packet
                .nodes
                .get(*child as usize)
                .is_some_and(|node| node.kind == 1 && node.name == literal)
        }) else {
            return;
        };
        if literals.peek().is_none() {
            packet.nodes[parent].children.retain(|node| *node != child);
            return;
        }
        parent = child as usize;
    }
}

fn is_hidden_literal(command: &str, paths: &[String]) -> bool {
    paths.iter().any(|path| {
        let mut path = path.split_ascii_whitespace().peekable();
        let mut command = command.split_ascii_whitespace();
        path.peek().is_some() && path.all(|literal| command.next() == Some(literal))
    })
}

fn filter_literal_suggestions(command: &str, paths: &[String], suggestions: &mut Vec<Suggestion>) {
    if paths.is_empty() {
        return;
    }
    if is_hidden_literal(command, paths) {
        suggestions.clear();
        return;
    }
    suggestions.retain(|suggestion| !is_hidden_literal(&suggestion.apply(command), paths));
}

impl Hub {
    /// Publish the frozen server token inventory before sending any command trees.
    pub fn set_selector_tokens(
        &mut self,
        tokens: Vec<horizon_client_core::registries::SelectorToken>,
    ) {
        self.selector_tokens = tokens;
    }

    /// Commands.sendCommands: the command tree for this player. Public because `Hub::set_mod_commands`, a function
    /// replacement's target (`tools/ci/fn-replace-check/lower`), calls it from a copy in another crate.
    pub fn send_commands(&self, i: usize) {
        let g = &self.guests[i];
        let mut packet = self.commands_packet(g.permission);
        for path in g.hidden_commands.iter().chain(&g.hidden_command_literals) {
            hide_literal(&mut packet, path);
        }
        if g.brand == "horizon" {
            g.send(&C::CustomPayload(
                horizon_protocol::packets::configuration::CustomPayload {
                    channel: crate::lan::commands::selector_sync::CHANNEL.into(),
                    data: crate::lan::commands::selector_sync::encode(&self.selector_tokens),
                },
            ));
        }
        g.send(&C::Commands(packet));
    }

    /// Applies the guest's literal-branch filters to server-side completions.
    pub fn filter_command_suggestions(
        &self,
        conn: u32,
        command: &str,
        suggestions: &mut Vec<Suggestion>,
    ) {
        if let Some(guest) = self.guest(conn) {
            filter_literal_suggestions(command, &guest.hidden_command_literals, suggestions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(name: &str, children: Vec<i32>) -> play::CommandNode {
        play::CommandNode {
            kind: 1,
            executable: false,
            restricted: false,
            children,
            redirect: None,
            name: name.to_owned(),
            parser: None,
            suggestions: None,
        }
    }

    #[test]
    fn hidden_literal_prunes_its_parent_edge_only() {
        let mut root = literal("", vec![1, 2]);
        root.kind = 0;
        let mut tree = play::Commands {
            nodes: vec![
                root,
                literal("pma", vec![3, 4]),
                literal("other", vec![3]),
                literal("reload", vec![]),
                literal("set", vec![5, 6]),
                literal("true", vec![]),
                literal("false", vec![]),
            ],
            root: 0,
        };
        hide_literal(&mut tree, "pma reload");
        hide_literal(&mut tree, "pma set false");
        hide_literal(&mut tree, "pma missing true");
        assert_eq!(tree.nodes[0].children, [1, 2]);
        assert_eq!(tree.nodes[1].children, [4]);
        assert_eq!(tree.nodes[2].children, [3]);
        assert_eq!(tree.nodes[4].children, [5]);
    }

    #[test]
    fn hidden_literal_suppresses_server_completion_with_token_boundaries() {
        let paths = vec!["pma reload".to_owned()];
        let suggestion = |start, end, text: &str| Suggestion {
            range: (start, end),
            text: text.to_owned(),
            tooltip: None,
            int_value: None,
        };
        let mut choices = vec![suggestion(4, 6, "reload"), suggestion(4, 6, "remove")];
        filter_literal_suggestions("pma re", &paths, &mut choices);
        assert_eq!(choices, [suggestion(4, 6, "remove")]);

        choices = vec![suggestion(11, 11, "config")];
        filter_literal_suggestions("pma reload ", &paths, &mut choices);
        assert!(choices.is_empty());

        choices = vec![suggestion(11, 11, "reload")];
        filter_literal_suggestions("pma remove ", &paths, &mut choices);
        assert_eq!(choices, [suggestion(11, 11, "reload")]);
    }
}
