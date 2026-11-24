//! PigeonMoe's negotiated wardrobe state, scoped to the remote session.
use super::Game;
use horizon_core::text::parse_java_uuid;
use serde::Deserialize;
use std::collections::HashMap;

pub(in crate::game) const CHANNEL: &str = "jianghu_wardrobe:appearance";

#[derive(Default, Deserialize)]
struct Slots {
    head: Option<String>,
    chest: Option<String>,
    legs: Option<String>,
    feet: Option<String>,
}

#[derive(Deserialize)]
struct Appearance {
    v: u8,
    player: String,
    slots: Slots,
}

#[derive(Default)]
pub(super) struct Wardrobe(HashMap<u128, [Option<String>; 4]>);

impl Wardrobe {
    fn receive(&mut self, bytes: &[u8]) {
        if bytes.len() > 1024 {
            return;
        }
        let Ok(Appearance { v: 1, player, slots }) = serde_json::from_slice(bytes) else {
            return;
        };
        let Some(uuid) = parse_java_uuid(&player) else {
            return;
        };
        let layers = [slots.head, slots.chest, slots.legs, slots.feet];
        if layers.iter().flatten().any(|id| !valid_asset(id)) {
            return;
        }
        if layers.iter().all(Option::is_none) {
            self.0.remove(&uuid);
        } else if self.0.len() < 1024 || self.0.contains_key(&uuid) {
            self.0.insert(uuid, layers);
        }
    }

    pub(super) fn forget(&mut self, uuid: u128) {
        self.0.remove(&uuid);
    }
}

fn valid_asset(id: &str) -> bool {
    id.strip_prefix("jianghu:").is_some_and(|path| {
        !path.is_empty()
            && path.len() <= 64
            && path.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    })
}

impl Game<'_> {
    pub(in crate::game) fn receive_wardrobe_appearance(&mut self, bytes: &[u8]) {
        if let Some(remote) = self.remote.as_mut() {
            remote.wardrobe.receive(bytes);
        }
    }

    /// None means the local avatar. Other players use the remote entity's profile.
    pub(in crate::game) fn wardrobe_loadout(&self, entity: Option<u32>) -> [Option<&str>; 4] {
        self.remote
            .as_ref()
            .and_then(|remote| {
                let uuid = match entity {
                    Some(id) => remote.player_uuid(id)?,
                    None => remote.profile(),
                };
                remote.wardrobe.0.get(&uuid)
            })
            .map(|layers| layers.each_ref().map(|layer| layer.as_deref()))
            .unwrap_or([None; 4])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independently_applies_and_clears_each_visible_slot() {
        let mut wardrobe = Wardrobe::default();
        let packet = |slots: serde_json::Value| {
            serde_json::to_vec(&serde_json::json!({
                "v": 1, "player": "00000000-0000-0000-0000-000000000003", "slots": slots
            })).unwrap()
        };
        wardrobe.receive(&packet(serde_json::json!({
            "head": "jianghu:qixiu", "chest": "jianghu:changge_ceremonial",
            "legs": null, "feet": "jianghu:yantianzong"
        })));
        assert_eq!(wardrobe.0[&3].each_ref().map(|s| s.as_deref()), [
            Some("jianghu:qixiu"), Some("jianghu:changge_ceremonial"), None, Some("jianghu:yantianzong")
        ]);
        wardrobe.receive(&packet(serde_json::json!({"head": "minecraft:diamond"})));
        assert!(wardrobe.0[&3][1].is_some());
        wardrobe.receive(&packet(serde_json::json!({"chest": null})));
        assert!(!wardrobe.0.contains_key(&3));
    }
}
