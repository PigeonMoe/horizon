//! Bukkit HuskSync 4.0.0's health, hunger and experience snapshot fields.

use std::fmt;

use serde::{
    Deserialize, Deserializer,
    de::{self, DeserializeOwned},
};
use serde_json::value::RawValue;

use crate::husksync_packed::PackedSnapshot;

pub const HEALTH_KEY: &str = "husksync:health";
pub const HUNGER_KEY: &str = "husksync:hunger";
pub const EXPERIENCE_KEY: &str = "husksync:experience";

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct HuskHealth {
    /// The inner Husk value. Applying it needs the destination's effective
    /// maximum, and PM-Sync's envelope can carry a separate health value.
    #[serde(deserialize_with = "java_double")]
    pub health: f64,
    /// HuskSync uses the destination maximum for a nonpositive scale.
    #[serde(deserialize_with = "java_double")]
    pub health_scale: f64,
    pub is_health_scaled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct HuskHunger {
    pub food_level: i32,
    #[serde(deserialize_with = "java_float")]
    pub saturation: f32,
    #[serde(deserialize_with = "java_float")]
    pub exhaustion: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct HuskExperience {
    pub total_experience: i32,
    pub exp_level: i32,
    #[serde(deserialize_with = "java_float")]
    pub exp_progress: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HuskVitals {
    pub health: HuskHealth,
    pub hunger: HuskHunger,
    pub experience: HuskExperience,
}

#[derive(Debug)]
pub enum VitalsError {
    MissingField(&'static str),
    InvalidJson {
        field: &'static str,
        error: serde_json::Error,
    },
    InvalidValue(&'static str),
}

impl fmt::Display for VitalsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(f, "missing HuskSync field {field}"),
            Self::InvalidJson { field, error } => {
                write!(f, "invalid HuskSync field {field}: {error}")
            }
            Self::InvalidValue(field) => write!(f, "invalid HuskSync value {field}"),
        }
    }
}

impl std::error::Error for VitalsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidJson { error, .. } => Some(error),
            _ => None,
        }
    }
}

impl PackedSnapshot {
    pub fn health(&self) -> Result<HuskHealth, VitalsError> {
        self.vital_field(HEALTH_KEY)
    }

    pub fn hunger(&self) -> Result<HuskHunger, VitalsError> {
        self.vital_field(HUNGER_KEY)
    }

    pub fn experience(&self) -> Result<HuskExperience, VitalsError> {
        self.vital_field(EXPERIENCE_KEY)
    }

    /// Stage all three required PM-Sync fields before changing any player
    /// state. Bukkit HuskSync writes total XP and level before checking
    /// progress, so applying the fields as they are read could leave a partial result.
    pub fn startup_vitals(&self) -> Result<HuskVitals, VitalsError> {
        let health = self.health()?;
        let hunger = self.hunger()?;
        let experience = self.experience()?;
        if health.health < 0.0 {
            return Err(VitalsError::InvalidValue("health"));
        }
        if experience.total_experience < 0 {
            return Err(VitalsError::InvalidValue("total_experience"));
        }
        if experience.exp_level < 0 {
            return Err(VitalsError::InvalidValue("exp_level"));
        }
        if !(0.0..=1.0).contains(&experience.exp_progress) {
            return Err(VitalsError::InvalidValue("exp_progress"));
        }
        Ok(HuskVitals {
            health,
            hunger,
            experience,
        })
    }

    fn vital_field<T: DeserializeOwned>(&self, field: &'static str) -> Result<T, VitalsError> {
        let raw = self
            .data()
            .get(field)
            .ok_or(VitalsError::MissingField(field))?;
        serde_json::from_str(raw).map_err(|error| VitalsError::InvalidJson { field, error })
    }
}

/// Gson's double adapter uses the decimal token at full precision.
fn java_double<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    let raw = <&RawValue>::deserialize(deserializer)?;
    raw.get()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| de::Error::custom("expected a finite JSON number"))
}

/// Gson reads a double before narrowing to a Java float.
fn java_float<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f32, D::Error> {
    let value = java_double(deserializer)? as f32;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(de::Error::custom(
            "number outside the finite Java float range",
        ))
    }
}
