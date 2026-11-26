//! The two names a command reads from an entity: `Entity.getDisplayName` for
//! the mobs command replies (the name as its team shows it, with the
//! `show_entity` hover and the UUID insertion) and `Nameable.getPlainTextName`
//! for the selector's `name=`.
use super::super::{Game, persist::OffLevel};
use super::Target;
use horizon_client::{mobs::Mob, text::Text};

impl Game<'_> {
    /// `Nameable.getPlainTextName`: `getName().getString()`, the name without
    /// the team's prefix and suffix that `getDisplayName` adds.
    pub(in crate::game) fn target_plain_name(&self, t: Target) -> String {
        match t {
            Target::Player => self.acting_player_name().to_owned(),
            Target::Guest(conn) => self
                .command_guest(conn)
                .map_or_else(String::new, |g| g.name.clone()),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| m.command_name().plain()),
            // No other entity's name is team-formatted.
            other => self.target_name(other).plain(),
        }
    }
    /// `Entity.getDisplayName` for the mob `id`: `PlayerTeam.formatNameForTeam`
    /// of `getName()`, then `withHoverEvent(createHoverEvent())` (a `ShowEntity`
    /// of the type, UUID and name) and `withInsertion(getStringUUID())`. A mob
    /// gets its UUID when it joins the level; one without keeps the bare name.
    pub(in crate::game) fn mob_command_name(&self, id: u32) -> Text {
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return Text::translate("entity.", &[]);
        };
        self.mob_command_display_name(m)
    }

    fn mob_command_display_name(&self, m: &Mob) -> Text {
        let name = m.command_name();
        let shown = match self.mob_team_of(m) {
            Some(team) => team.formatted_name(name.clone()),
            None => name.clone(),
        };
        let Some(uuid) = self.persist.uuid_of(m.id) else {
            return shown;
        };
        let uuid = horizon_client::save::uuid_text(uuid);
        let kind = horizon_client::command::args::normalize(&m.kind.id);
        shown.with_component_fields(vec![
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": kind, "uuid": uuid,
                    "name": name.component()}),
            ),
            ("insertion".into(), uuid.into()),
        ])
    }

    /// The display name of a rider's retained mount for `/ride dismount`.
    pub(super) fn retained_vehicle_name(&self, id: u32) -> Option<Text> {
        let kind = match self.persist.off_level.get(&id)? {
            OffLevel::Mob(mob) => return Some(self.mob_command_display_name(mob)),
            OffLevel::Boat(boat) => boat.entity_type(),
            OffLevel::Cart(cart) => cart.kind.id().into(),
        };
        if let Some(custom) = self.persist_raw(id).and_then(|r| r.get("CustomName")) {
            return Some(horizon_client::scoreboard::comp_text(
                &horizon_client::scoreboard::tag_to_json(custom),
            ));
        }
        Some(Text::translate(
            &format!("entity.{}", kind.replace(':', ".")),
            &[],
        ))
    }
}
