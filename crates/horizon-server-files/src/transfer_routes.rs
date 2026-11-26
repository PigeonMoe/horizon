//! Backend identifiers shared by the supervisor configuration and the game.

use std::collections::BTreeSet;

pub const BACKEND_ID_MAX_BYTES: usize = 96;

pub fn valid_backend_id(id: &str) -> bool {
    (1..=BACKEND_ID_MAX_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

pub fn validate(
    backend_id: Option<&str>,
    allowed_sources: &BTreeSet<String>,
) -> Result<(), String> {
    if backend_id.is_none() && !allowed_sources.is_empty() {
        return Err("allowed_sources needs backend_id".into());
    }
    if let Some(id) = backend_id {
        if !valid_backend_id(id) {
            return Err(format!("invalid transfer backend_id {id:?}"));
        }
        if allowed_sources.contains(id) {
            return Err("allowed_sources includes this backend".into());
        }
    }
    if let Some(id) = allowed_sources.iter().find(|id| !valid_backend_id(id)) {
        return Err(format!("invalid transfer allowed source {id:?}"));
    }
    Ok(())
}
