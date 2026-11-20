//! Management cards for loaded packages. The operator installs the TOML alongside the server.
//! The browser sends a package and action ID, and the supervisor resolves the fixed console command.

use crate::{
    admin::{running, runtime_mods},
    api::Shared,
    http::Response,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashSet, fs::File, io::Read, path::Path};

mod config;

const MAX_MANIFEST: u64 = 64 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Card {
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    actions: Vec<Action>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<config::Editor>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Action {
    id: String,
    label: String,
    #[serde(skip_serializing)]
    command: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    plugin: String,
    action: String,
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn label(value: &str, max_chars: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= max_chars
        && !value.chars().any(char::is_control)
}

impl Card {
    fn load(dir: &Path, plugin: &str) -> Option<Self> {
        if !valid_id(plugin) {
            return None;
        }
        let path = dir.join("panel-plugins").join(format!("{plugin}.toml"));
        if !path.symlink_metadata().ok()?.is_file() {
            return None;
        }
        let mut text = String::new();
        File::open(path)
            .ok()?
            .take(MAX_MANIFEST + 1)
            .read_to_string(&mut text)
            .ok()?;
        if text.len() as u64 > MAX_MANIFEST {
            return None;
        }
        Self::parse(&text, plugin)
    }

    fn parse(text: &str, plugin: &str) -> Option<Self> {
        let card: Self = toml::from_str(text).ok()?;
        let mut action_ids = HashSet::new();
        let valid = label(&card.title, 80)
            && (card.description.is_empty() || label(&card.description, 200))
            && card.config.as_ref().is_none_or(config::Editor::valid)
            && card.actions.len() <= 16
            && card.actions.iter().all(|action| {
                valid_id(&action.id)
                    && action_ids.insert(&action.id)
                    && label(&action.label, 60)
                    && action.command.len() <= 256
                    && !action.command.chars().any(char::is_control)
                    && action.command.split_ascii_whitespace().next() == Some(plugin)
            });
        valid.then_some(card)
    }
}

pub(super) fn settings(shared: &Shared, plugin: &str, body: Option<&[u8]>) -> Response {
    config::handle(shared, plugin, body)
}

pub(super) fn list(shared: &Shared) -> Response {
    let inventory = match running(shared).and_then(|mut game| runtime_mods(&mut game)) {
        Ok(inventory) => inventory,
        Err(response) => return response,
    };
    let plugins: Vec<_> = inventory
        .into_iter()
        .map(|package| {
            let card = Card::load(&shared.dir, &package.id);
            json!({ "id": package.id, "version": package.version, "card": card })
        })
        .collect();
    Response::json(200, &json!({ "plugins": plugins }))
}

pub(super) fn run(shared: &Shared, body: &[u8]) -> Response {
    let Ok(selected) = serde_json::from_slice::<Selection>(body) else {
        return Response::error(400, "the body needs plugin and action IDs");
    };
    let Some(action) = Card::load(&shared.dir, &selected.plugin).and_then(|card| {
        card.actions
            .into_iter()
            .find(|action| action.id == selected.action)
    }) else {
        return Response::error(404, "this plugin action isn't registered");
    };
    let mut game = match running(shared) {
        Ok(game) => game,
        Err(response) => return response,
    };
    match runtime_mods(&mut game) {
        Ok(inventory)
            if inventory
                .iter()
                .any(|package| package.id == selected.plugin) => {}
        Ok(_) => return Response::error(404, "this plugin isn't loaded"),
        Err(response) => return response,
    }
    match game.ask(&format!("console {}", action.command)) {
        Ok(reply) => {
            crate::supervisor::note(
                shared,
                "INFO",
                &format!(
                    "Panel plugin action: {} {}",
                    selected.plugin, selected.action
                ),
            );
            Response::json(200, &reply)
        }
        Err(e) => Response::error(409, &format!("the game didn't answer: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_expose_labels_and_restrict_actions_to_the_owning_command() {
        let text = r#"
            title = "Example plugin"
            [[actions]]
            id = "status"
            label = "Read status"
            command = "example status"
        "#;
        let card = Card::parse(text, "example").unwrap();
        let value = serde_json::to_value(card).unwrap();
        assert_eq!(
            value["actions"],
            json!([{ "id": "status", "label": "Read status" }])
        );
        assert!(Card::parse(text, "another_plugin").is_none());
        assert!(
            Card::parse(
                &text.replace("example status", "example status\\nstop"),
                "example"
            )
            .is_none()
        );
        assert!(Card::parse(&text.replace("example status", "stop"), "example").is_none());
        assert!(Card::load(Path::new("."), "../elsewhere").is_none());
    }
}
