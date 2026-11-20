//! Selected scalar settings from a locally installed plugin panel manifest.

use super::{label, valid_id};
use crate::{api::Shared, http::Response};
use horizon_platform::server_files::private_file::write_private;
use horizon_server_launch::config::Config;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
};
use toml_edit::{DocumentMut, Item};
use yaml_rust2::{Yaml, YamlLoader};

const MAX_CONFIG: u64 = 256 * 1024;
static EDIT: Mutex<()> = Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Editor {
    #[serde(skip_serializing)]
    file: String,
    #[serde(default, skip_serializing)]
    world_override: bool,
    note: String,
    fields: Vec<Field>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Field {
    id: String,
    #[serde(skip_serializing)]
    key: String,
    label: String,
    #[serde(default)]
    description: String,
    kind: Kind,
    min: Option<i64>,
    max: Option<i64>,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Kind {
    Boolean,
    Integer,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    revision: String,
    changes: BTreeMap<String, Value>,
}

fn safe_component(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

impl Editor {
    pub(super) fn valid(&self) -> bool {
        let parts: Vec<_> = self.file.split('/').collect();
        let allowed = match parts.as_slice() {
            ["config", file] => file.ends_with(".toml"),
            ["plugins", _, "config.yml" | "config.yaml"] => !self.world_override,
            _ => false,
        };
        let mut ids = HashSet::new();
        let mut keys = HashSet::new();
        allowed
            && parts.iter().all(|part| safe_component(part))
            && label(&self.note, 200)
            && (1..=32).contains(&self.fields.len())
            && self.fields.iter().all(|field| {
                valid_id(&field.id)
                    && ids.insert(&field.id)
                    && keys.insert(&field.key)
                    && field.key.len() <= 128
                    && field.key.split('.').all(|segment| {
                        safe_component(segment) && segment.len() <= 64
                    })
                    && label(&field.label, 60)
                    && (field.description.is_empty() || label(&field.description, 200))
                    && match field.kind {
                        Kind::Boolean => field.min.is_none() && field.max.is_none(),
                        Kind::Integer => matches!((field.min, field.max), (Some(min), Some(max)) if min <= max && min >= i64::from(i32::MIN) && max <= i64::from(i32::MAX)),
                    }
            })
    }

    fn path(&self, dir: &Path) -> Result<PathBuf, &'static str> {
        if self.world_override {
            let world = Config::load(dir).map_err(|_| "could not read the current world")?.world.name;
            let name = self.file.rsplit('/').next().unwrap();
            let override_path = Path::new(&world).join("serverconfig").join(name);
            match regular_file(dir, &override_path) {
                Ok(Some(path)) => return Ok(path),
                Err(()) => return Err("the plugin configuration path is unavailable"),
                Ok(None) => {}
            }
        }
        regular_file(dir, Path::new(&self.file))
            .ok()
            .flatten()
            .ok_or("the plugin configuration file is unavailable")
    }
}

// A manifest is local operator metadata. The browser never supplies file names or keys.
// Refuse indirect paths before even reading a file, including world overrides.
fn regular_file(dir: &Path, relative: &Path) -> Result<Option<PathBuf>, ()> {
    use std::path::Component;
    let mut path = dir.to_owned();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        let Component::Normal(name) = part else { return Err(()) };
        path.push(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(()),
        };
        if (parts.peek().is_some() && !metadata.is_dir())
            || (parts.peek().is_none() && (!metadata.is_file() || metadata.len() > MAX_CONFIG))
        {
            return Err(());
        }
    }
    Ok(Some(path))
}

fn read_text(path: &Path) -> Result<String, &'static str> {
    let mut text = String::new();
    File::open(path)
        .and_then(|file| file.take(MAX_CONFIG + 1).read_to_string(&mut text))
        .map_err(|_| "could not read the plugin configuration")?;
    if text.len() as u64 > MAX_CONFIG {
        return Err("the plugin configuration is too large");
    }
    Ok(text)
}

fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text))
}

fn yaml_document(text: &str) -> Result<Yaml, &'static str> {
    let mut documents = YamlLoader::load_from_str(text).map_err(|_| "the plugin YAML is invalid")?;
    if documents.len() != 1 {
        return Err("the plugin YAML must contain one document");
    }
    Ok(documents.pop().unwrap())
}

fn current(kind: Kind, boolean: Option<bool>, integer: Option<i64>) -> Option<Value> {
    match kind {
        Kind::Boolean => boolean.map(Value::from),
        Kind::Integer => integer.filter(|n| i32::try_from(*n).is_ok()).map(Value::from),
    }
}

fn toml_get<'a>(doc: &'a DocumentMut, key: &str) -> Option<&'a Item> {
    key.split('.').try_fold(doc.as_item(), |item, part| item.get(part))
}

fn toml_get_mut<'a>(doc: &'a mut DocumentMut, key: &str) -> Option<&'a mut Item> {
    key.split('.').try_fold(doc.as_item_mut(), |item, part| item.get_mut(part))
}

fn yaml_get<'a>(doc: &'a Yaml, key: &str) -> &'a Yaml {
    key.split('.').fold(doc, |item, part| &item[part])
}

fn yaml_get_mut<'a>(doc: &'a mut Yaml, key: &str) -> Option<&'a mut Yaml> {
    key.split('.').try_fold(doc, |item, part| match item {
        Yaml::Hash(table) => table.get_mut(&Yaml::String(part.into())),
        _ => None,
    })
}

/// Replace one plain scalar in block-mapping YAML, preserving comments and other fields. The
/// caller compares the reparsed document to the expected document before publishing the result.
fn replace_yaml_scalar(text: &mut String, key: &str, value: &Value) -> Option<()> {
    let mut parents: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0;
    let mut found = None;
    for line in text.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let raw = line.trim_start_matches(' ');
        let indent = line.len() - raw.len();
        if raw.trim().is_empty() || raw.starts_with('#') {
            continue;
        }
        parents.retain(|(level, _)| *level < indent);
        let Some((segment, tail)) = raw.split_once(':') else { continue };
        if !safe_component(segment) {
            continue;
        }
        let scalar = tail.trim_start_matches(' ');
        let first = scalar.split_ascii_whitespace().next();
        if first.is_none_or(|word| word.starts_with('#')) {
            parents.push((indent, segment));
        } else if parents.iter().map(|(_, label)| *label).chain([segment]).eq(key.split('.')) {
            let word = first?;
            if found.is_some() || !matches!(word, "true" | "false") && word.parse::<i64>().is_err() {
                return None;
            }
            let start = start + line.len() - scalar.len();
            found = Some(start..start + word.len());
        }
    }
    text.replace_range(found?, &value.to_string());
    Some(())
}

fn values(toml: bool, text: &str, editor: &Editor) -> Result<BTreeMap<String, Value>, &'static str> {
    let values = if toml {
        let doc = text.parse::<DocumentMut>().map_err(|_| "the plugin TOML is invalid")?;
        editor.fields.iter().filter_map(|field| {
            let item = toml_get(&doc, &field.key)?;
            Some((field.id.clone(), current(field.kind, item.as_bool(), item.as_integer())?))
        }).collect()
    } else {
        let doc = yaml_document(text)?;
        editor.fields.iter().filter_map(|field| {
            let item = yaml_get(&doc, &field.key);
            Some((field.id.clone(), current(field.kind, item.as_bool(), item.as_i64())?))
        }).collect()
    };
    Ok(values)
}

fn patch_text(
    toml: bool,
    text: &str,
    editor: &Editor,
    changes: BTreeMap<String, Value>,
) -> Result<String, &'static str> {
    let current = values(toml, text, editor)?;
    let mut edits = Vec::new();
    for (id, value) in changes {
        let field = editor.fields.iter().find(|field| field.id == id && current.contains_key(&id))
            .ok_or("this plugin setting is not editable")?;
        let valid = match field.kind {
            Kind::Boolean => value.is_boolean(),
            Kind::Integer => value.as_i64().is_some_and(|n| field.min.unwrap() <= n && n <= field.max.unwrap()),
        };
        if !valid {
            return Err("the plugin setting has an invalid value");
        }
        if current[&id] != value {
            edits.push((&field.key, value));
        }
    }
    if toml {
        let mut doc = text.parse::<DocumentMut>().map_err(|_| "the plugin TOML is invalid")?;
        for (key, value) in edits {
            let old = toml_get_mut(&mut doc, key).and_then(Item::as_value_mut).unwrap();
            let mut new = match value {
                Value::Bool(b) => toml_edit::Value::from(b),
                _ => toml_edit::Value::from(value.as_i64().unwrap()),
            };
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        Ok(doc.to_string())
    } else {
        let mut expected = yaml_document(text)?;
        let mut changed = text.to_string();
        for (key, value) in edits {
            *yaml_get_mut(&mut expected, key).unwrap() = match value {
                Value::Bool(b) => Yaml::Boolean(b),
                _ => Yaml::Integer(value.as_i64().unwrap()),
            };
            replace_yaml_scalar(&mut changed, key, &value)
                .ok_or("this YAML setting is not a plain value on its own line")?;
        }
        if yaml_document(&changed)? != expected {
            return Err("the plugin YAML changed in an unexpected way");
        }
        Ok(changed)
    }
}

pub(super) fn handle(shared: &Shared, plugin: &str, body: Option<&[u8]>) -> Response {
    let Some(editor) = super::Card::load(&shared.dir, plugin).and_then(|card| card.config) else {
        return Response::error(404, "this plugin has no editable settings");
    };
    let _edit = EDIT.lock().unwrap();
    let path = match editor.path(&shared.dir) {
        Ok(path) => path,
        Err(e) => return Response::error(409, e),
    };
    let text = match read_text(&path) {
        Ok(text) => text,
        Err(e) => return Response::error(409, e),
    };
    let toml = path.extension().is_some_and(|extension| extension == "toml");
    let version = revision(&text);
    let Some(body) = body else {
        return match values(toml, &text, &editor) {
            Ok(values) => Response::json(200, &json!({ "revision": version, "values": values })),
            Err(e) => Response::error(409, e),
        };
    };
    let Ok(update) = serde_json::from_slice::<Patch>(body) else {
        return Response::error(400, "the body needs a revision and settings");
    };
    if update.changes.is_empty() || update.changes.len() > editor.fields.len() {
        return Response::error(400, "no valid plugin settings were submitted");
    }
    if update.revision != version {
        return Response::error(409, "the plugin settings changed; reopen the editor to get the latest values");
    }
    let changed = match patch_text(toml, &text, &editor, update.changes) {
        Ok(changed) => changed,
        Err(e) => return Response::error(400, e),
    };
    if changed != text {
        if !matches!(read_text(&path), Ok(now) if now == text) {
            return Response::error(409, "the plugin settings changed; reopen the editor to get the latest values");
        }
        if write_private(&path, changed.as_bytes()).is_err() {
            return Response::error(500, "could not save the plugin configuration");
        }
        crate::supervisor::note(shared, "INFO", &format!("Panel plugin settings saved: {plugin}"));
    }
    Response::json(200, &json!({ "revision": revision(&changed) }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(file: &str) -> Editor {
        toml::from_str(&format!(r#"
            file = "{file}"
            note = "Reload after saving."
            [[fields]]
            id = "limit"
            key = "Settings.Limit"
            label = "Limit"
            kind = "integer"
            min = 1
            max = 20
        "#)).unwrap()
    }

    #[test]
    fn editing_preserves_toml_and_yaml_comments_and_unrelated_values() {
        for (is_toml, file, original, old, new) in [
            (true, "config/example.toml", "[Settings]\nLimit = 10 # limit\nOther = false\n", "= 10", "= 12"),
            (false, "plugins/Example/config.yml", "# options\nSettings:\n  Limit: 10 # limit\n  Other: false\n", ": 10", ": 12"),
        ] {
            let editor = editor(file);
            assert!(editor.valid());
            assert_eq!(values(is_toml, original, &editor).unwrap(), BTreeMap::from([("limit".into(), json!(10))]));
            let changed = patch_text(is_toml, original, &editor, BTreeMap::from([("limit".into(), json!(12))])).unwrap();
            assert_eq!(changed, original.replace(old, new));
            for changes in [BTreeMap::from([("limit".into(), json!(21))]), BTreeMap::from([("other".into(), json!(false))])] {
                assert!(patch_text(is_toml, original, &editor, changes).is_err());
            }
        }
        let editor = editor("plugins/Example/config.yml");
        assert!(patch_text(false, "Settings: {Limit: 10}\n", &editor, BTreeMap::from([("limit".into(), json!(12))])).is_err());
    }

    #[test]
    fn editor_refuses_an_old_revision_and_limits_access_to_registered_fields() {
        let dir = std::env::temp_dir().join(format!("horizon-panel-config-{}", crate::api::random_token()));
        fs::create_dir_all(dir.join("panel-plugins")).unwrap();
        fs::create_dir_all(dir.join("config")).unwrap();
        let path = dir.join("config/example.toml");
        let original = "[Settings]\nLimit = 10\nUnlisted = 123456\n";
        fs::write(&path, original).unwrap();
        let manifest = r#"
            title = "Example"
            [config]
            file = "config/example.toml"
            note = "Reload after saving."
            [[config.fields]]
            id = "limit"
            key = "Settings.Limit"
            label = "Limit"
            kind = "integer"
            min = 1
            max = 20
        "#;
        fs::write(dir.join("panel-plugins/example.toml"), manifest).unwrap();
        let shared = Shared::with_install(&dir, None);
        let response = handle(&shared, "example", None);
        assert_eq!(response.status, 200);
        assert_eq!(serde_json::from_str::<Value>(&response.body).unwrap(), json!({ "revision": revision(original), "values": {"limit": 10} }));
        let update = json!({ "revision": revision(original), "changes": {"limit": 12} }).to_string();
        assert_eq!(handle(&shared, "example", Some(update.as_bytes())).status, 200);
        assert_eq!(handle(&shared, "example", Some(update.as_bytes())).status, 409);
        assert_eq!(fs::read_to_string(&path).unwrap(), original.replace("= 10", "= 12"));
        assert!(!editor("config/../server.toml").valid());
        assert!(regular_file(&dir, Path::new("../server.toml")).is_err());
        #[cfg(unix)]
        {
            let link = dir.join("config/linked.toml");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(regular_file(&dir, Path::new("config/linked.toml")).is_err());
        }
        fs::remove_dir_all(dir).unwrap();
    }
}
