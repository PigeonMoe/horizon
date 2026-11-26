use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_mod_event(&self, topic: &str, payload_json: &str) {
        self.fire_mod_event(topic, payload_json);
    }
}
