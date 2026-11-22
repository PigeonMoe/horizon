//! The immutable language used by the Java server's `Language.loadDefault`.
use super::Game;
use horizon_core::prepared_files;
use std::{collections::HashMap, path::Path};

pub(super) fn load(vanilla_root: &Path) -> Option<HashMap<String, String>> {
    let lang = vanilla_root.join("assets/minecraft/lang");
    let english = prepared_files::read(&lang.join("en_us.json")).ok()?;
    let mut templates: HashMap<String, String> = serde_json::from_slice(&english).ok()?;
    for template in templates.values_mut().filter(|value| value.contains('%')) {
        *template = normalize_numeric_formats(template);
    }
    let deprecated = prepared_files::read(&lang.join("deprecated.json")).ok()?;
    serde_json::from_slice::<serde_json::Value>(&deprecated).ok()?;
    horizon_client_core::lang::apply_deprecated(&mut templates, &deprecated);
    Some(templates)
}

/// `Language.loadFromJson` replaces numeric formats with string placeholders.
fn normalize_numeric_formats(template: &str) -> String {
    let mut output = String::with_capacity(template.len());
    let mut remaining = template;
    while let Some(percent) = remaining.find('%') {
        output.push_str(&remaining[..=percent]);
        remaining = &remaining[percent + 1..];
        let after_index = remaining.trim_start_matches(|ch: char| ch.is_ascii_digit());
        let (index, width) = match after_index.strip_prefix('$') {
            Some(width) if after_index.len() != remaining.len() => {
                (&remaining[..remaining.len() - width.len()], width)
            }
            _ => ("", remaining),
        };
        let format = width.trim_start_matches(|ch: char| ch.is_ascii_digit() || ch == '.');
        if let Some(rest) = format.strip_prefix(['d', 'f']) {
            output.push_str(index);
            output.push('s');
            remaining = rest;
        }
    }
    output.push_str(remaining);
    output
}

impl Game<'_> {
    pub(super) fn server_default_translation(&self, key: &str) -> Option<Option<String>> {
        Some(self.res.core.server_language.as_ref()?.get(key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{execution_role::ExecutionRole, native_entity_refs::tests::with_game_role};

    #[test]
    fn loads_only_a_complete_vanilla_table_and_uses_java_format_and_rename_rules() {
        let root = tempfile::tempdir().unwrap();
        let lang = root.path().join("assets/minecraft/lang");
        std::fs::create_dir_all(&lang).unwrap();
        std::fs::write(
            lang.join("en_us.json"),
            r#"{"old":"%2$1.3f / %4d / %% / %s","removed":"Removed"}"#,
        )
        .unwrap();
        assert!(load(root.path()).is_none());
        std::fs::write(
            lang.join("deprecated.json"),
            r#"{"removed":["removed"],"renamed":{"old":"current"}}"#,
        )
        .unwrap();
        let templates = load(root.path()).unwrap();
        assert_eq!(
            templates,
            HashMap::from([("current".into(), "%2$s / %s / %% / %s".into())])
        );
    }

    #[test]
    #[ignore = "daily"]
    fn dedicated_server_uses_its_prepared_26_3_english_even_in_a_client_locale_scope() {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").unwrap().into();
        with_game_role(root, ExecutionRole::DedicatedServer, |game| {
            let client_language = horizon_client_core::lang::Language::of(
                "zh_cn",
                [("item.minecraft.apple".into(), "苹果".into())],
            );
            horizon_client_core::lang::scoped(client_language, || {
                let host: &dyn mod_api::ServerHost = game;
                assert_eq!(horizon_core::VANILLA_VERSION, "26.3");
                assert_eq!(
                    host.server_default_translation("item.minecraft.apple"),
                    Some(Some("Apple".into()))
                );
                assert_eq!(
                    host.server_default_translation("advancements.story.iron_tools.title"),
                    Some(Some("Isn't It Iron Pick?".into()))
                );
                assert_eq!(
                    host.server_default_translation("warehouse.item.missing"),
                    Some(None)
                );
            });
        });
    }
}
