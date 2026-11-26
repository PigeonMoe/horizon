//! The Bukkit HuskSync 4.0.0 potion effect and attribute fields in Packed data.

use std::{collections::BTreeSet, fmt};

use horizon_core::{identifier::Identifier, text::parse_java_uuid};
use horizon_world::attributes::Operation;
use serde::{Deserialize, de::DeserializeOwned};

use crate::husksync_packed::{PackedSnapshot, java_double};

pub const POTION_EFFECTS_KEY: &str = "husksync:potion_effects";
pub const ATTRIBUTES_KEY: &str = "husksync:attributes";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct HuskPotionEffect {
    #[serde(rename = "type")]
    pub effect_type: String,
    pub amplifier: i32,
    pub duration: i32,
    pub is_ambient: bool,
    pub show_particles: bool,
    pub has_icon: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HuskAttribute {
    pub name: String,
    #[serde(rename = "baseValue", deserialize_with = "java_double")]
    pub base_value: f64,
    pub modifiers: Vec<HuskAttributeModifier>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct HuskAttributeModifier {
    pub name: String,
    #[serde(deserialize_with = "java_double")]
    pub amount: f64,
    pub operation: i32,
    /// Retained from older HuskSync records. Current apply uses the group.
    #[serde(default)]
    pub equipment_slot: i32,
    #[serde(default = "default_slot_group")]
    pub equipment_slot_group: String,
    pub uuid: Option<String>,
}

impl HuskAttributeModifier {
    /// Bukkit's ADD_NUMBER, ADD_SCALAR, and MULTIPLY_SCALAR_1 ordinals.
    pub fn native_operation(&self) -> Option<Operation> {
        let index = usize::try_from(self.operation).ok()?;
        Operation::ALL.get(index).copied()
    }

    /// HuskSync resolves the Bukkit slot name case-insensitively and uses
    /// `any` for an unknown group. The saved spelling remains available.
    pub fn effective_slot_group(&self) -> &'static str {
        [
            "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
            "saddle",
        ]
        .into_iter()
        .find(|group| self.equipment_slot_group.eq_ignore_ascii_case(group))
        .unwrap_or("any")
    }
}

fn default_slot_group() -> String {
    "any".to_owned()
}

#[derive(Clone, Debug, PartialEq)]
pub struct HuskEffectsAndAttributes {
    pub effects: Vec<HuskPotionEffect>,
    pub attributes: Vec<HuskAttribute>,
}

#[derive(Debug)]
pub enum EffectsAttributesError {
    MissingField(&'static str),
    InvalidJson {
        field: &'static str,
        error: serde_json::Error,
    },
    InvalidIdentifier {
        field: &'static str,
        id: String,
    },
    DuplicateIdentifier {
        field: &'static str,
        id: String,
    },
    ModifierOperation(i32),
    ModifierUuid(String),
    PmaModifier(String),
}

impl fmt::Display for EffectsAttributesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing HuskSync field {field}"),
            Self::InvalidJson { field, error } => {
                write!(f, "invalid HuskSync field {field}: {error}")
            }
            Self::InvalidIdentifier { field, id } => {
                write!(f, "invalid HuskSync {field} identifier {id:?}")
            }
            Self::DuplicateIdentifier { field, id } => {
                write!(f, "duplicate HuskSync {field} identifier {id:?}")
            }
            Self::ModifierOperation(operation) => {
                write!(f, "unknown HuskSync attribute operation {operation}")
            }
            Self::ModifierUuid(uuid) => {
                write!(f, "invalid HuskSync attribute modifier UUID {uuid:?}")
            }
            Self::PmaModifier(id) => write!(
                f,
                "PM-Attibute derived modifier appeared in the snapshot: {id:?}"
            ),
        }
    }
}

impl std::error::Error for EffectsAttributesError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl PackedSnapshot {
    /// The saved effect list, including raw flags and values.
    pub fn effects(&self) -> Result<Vec<HuskPotionEffect>, EffectsAttributesError> {
        self.effect_attribute_field(POTION_EFFECTS_KEY)
    }

    pub fn attributes(&self) -> Result<Vec<HuskAttribute>, EffectsAttributesError> {
        #[derive(Deserialize)]
        struct Attributes {
            attributes: Vec<HuskAttribute>,
        }
        let field: Attributes = self.effect_attribute_field(ATTRIBUTES_KEY)?;
        Ok(field.attributes)
    }

    /// Stage the two required fields. HuskSync discards ambient effects on
    /// read. The native effect constructor bounds amplifier to an unsigned
    /// byte. Registry lookup and actual application belong to the destination.
    pub fn startup_effects_and_attributes(
        &self,
    ) -> Result<HuskEffectsAndAttributes, EffectsAttributesError> {
        let mut effects = self.effects()?;
        let attributes = self.attributes()?;
        effects.retain(|effect| !effect.is_ambient);
        let mut effect_ids = BTreeSet::new();
        for effect in &mut effects {
            validate_id(&mut effect_ids, &effect.effect_type, "potion effect")?;
            effect.amplifier = effect.amplifier.clamp(0, i32::from(u8::MAX));
        }

        let mut attribute_ids = BTreeSet::new();
        for attribute in &attributes {
            validate_id(&mut attribute_ids, &attribute.name, "attribute")?;
            let mut modifier_ids = BTreeSet::new();
            for modifier in &attribute.modifiers {
                validate_id(&mut modifier_ids, &modifier.name, "attribute modifier")?;
                if modifier.name.starts_with("pmattibute:") {
                    return Err(EffectsAttributesError::PmaModifier(modifier.name.clone()));
                }
                if modifier.native_operation().is_none() {
                    return Err(EffectsAttributesError::ModifierOperation(
                        modifier.operation,
                    ));
                }
                if let Some(uuid) = &modifier.uuid
                    && parse_java_uuid(uuid).is_none()
                {
                    return Err(EffectsAttributesError::ModifierUuid(uuid.clone()));
                }
            }
        }
        Ok(HuskEffectsAndAttributes {
            effects,
            attributes,
        })
    }

    fn effect_attribute_field<T: DeserializeOwned>(
        &self,
        field: &'static str,
    ) -> Result<T, EffectsAttributesError> {
        let raw = self
            .data()
            .get(field)
            .ok_or(EffectsAttributesError::MissingField(field))?;
        serde_json::from_str(raw)
            .map_err(|error| EffectsAttributesError::InvalidJson { field, error })
    }
}

fn validate_id<'a>(
    seen: &mut BTreeSet<&'a str>,
    id: &'a str,
    field: &'static str,
) -> Result<(), EffectsAttributesError> {
    if Identifier::parse_namespaced(id).is_none() {
        return Err(EffectsAttributesError::InvalidIdentifier {
            field,
            id: id.to_owned(),
        });
    }
    if !seen.insert(id) {
        return Err(EffectsAttributesError::DuplicateIdentifier {
            field,
            id: id.to_owned(),
        });
    }
    Ok(())
}
