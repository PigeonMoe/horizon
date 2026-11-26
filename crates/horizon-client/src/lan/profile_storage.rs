//! Durability boundary for the held player's validated profile sections.
//!
//! The immutable bytes are written before the player file references them.
//! Unreferenced bytes from an interrupted placement have no authority.

use super::{playerdata, profile_sections::MAX_PROFILE_BYTES};
use horizon_core::{
    identifier::Identifier,
    nbt::{Compound, Tag},
};
use mod_api::TransferProfileReadError;
use sha2::{Digest, Sha256};
use std::{
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

const MARKERS: &str = "HorizonTransferProfiles";

pub(super) struct PreparedProfiles {
    pub owner: u128,
    pub handover: u128,
    pub connection: u32,
    pub profiles: Vec<super::profile_sections::Section>,
    pub player_data: Compound,
    pub fingerprint: [u8; 32],
}

/// Native-owned marker metadata, outside the `NeoForgeData` mod compound.
struct Marker {
    generation: u128,
    schema: u32,
    len: usize,
    sha256: [u8; 32],
}

impl Marker {
    fn for_payload(generation: u128, schema: u32, payload: &[u8]) -> Self {
        Self {
            generation,
            schema,
            len: payload.len(),
            sha256: Sha256::digest(payload).into(),
        }
    }

    fn to_tag(&self) -> Tag {
        Tag::compound([
            (
                "generation",
                Tag::ByteArray(self.generation.to_be_bytes().to_vec()),
            ),
            ("schema", Tag::Long(i64::from(self.schema))),
            ("length", Tag::Int(self.len as i32)),
            ("sha256", Tag::ByteArray(self.sha256.to_vec())),
        ])
    }

    fn from_tag(tag: &Tag) -> Option<Self> {
        let Tag::ByteArray(generation) = tag.get("generation")? else {
            return None;
        };
        let Tag::Long(schema) = tag.get("schema")? else {
            return None;
        };
        let Tag::Int(len) = tag.get("length")? else {
            return None;
        };
        let Tag::ByteArray(sha256) = tag.get("sha256")? else {
            return None;
        };
        let schema = u32::try_from(*schema).ok().filter(|v| *v > 0)?;
        let len = usize::try_from(*len)
            .ok()
            .filter(|v| (1..=MAX_PROFILE_BYTES).contains(v))?;
        Some(Self {
            generation: u128::from_be_bytes(generation.as_slice().try_into().ok()?),
            schema,
            len,
            sha256: sha256.as_slice().try_into().ok()?,
        })
    }

    fn path(&self, directory: &Path, owner: u128, id: &Identifier) -> PathBuf {
        let key = Sha256::digest(id.to_string());
        directory.join(format!(
            "{owner:032x}-{:032x}-{key:x}.hprofile",
            self.generation
        ))
    }

    fn verify_metadata(&self, file: &Path) -> std::io::Result<()> {
        let metadata = std::fs::symlink_metadata(file)?;
        if !metadata.is_file() || metadata.len() != self.len as u64 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "invalid profile length",
            ));
        }
        Ok(())
    }

    fn verify_read(&self, file: &Path) -> std::io::Result<Vec<u8>> {
        self.verify_metadata(file)?;
        let mut bytes = Vec::with_capacity(self.len);
        std::fs::File::open(file)?
            .take(self.len as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() != self.len || Sha256::digest(&bytes).as_slice() != self.sha256 {
            return Err(std::io::Error::new(
                ErrorKind::InvalidData,
                "invalid profile digest",
            ));
        }
        Ok(bytes)
    }

    fn make_durable(&self, file: &Path, payload: &[u8]) -> Result<(), String> {
        match self.verify_read(file) {
            // A previous rename may have returned an error while syncing its directory.
            Ok(_) => {
                #[cfg(unix)]
                if let Some(parent) = file.parent() {
                    std::fs::File::open(parent)
                        .and_then(|dir| dir.sync_all())
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                crate::save::write_durable(file, payload).map_err(|error| error.to_string())
            }
            // A retry for this generation may never rewrite different bytes.
            Err(error) => Err(error.to_string()),
        }
    }
}

impl PreparedProfiles {
    /// Put the new markers in the candidate's extra data after every immutable
    /// section has been made durable. The caller publishes the returned set
    /// through the normal durable player file write before the player is placed.
    pub(super) fn make_durable(&self, world: &Path) -> Result<Compound, String> {
        let directory = playerdata::directory(world);
        // `players/data` may have been created for this owner. Sync both directory
        // links before their contents can become the player's active checkpoint.
        #[cfg(unix)]
        for parent in [world, &world.join("players")] {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| error.to_string())?;
        }
        let mut markers = Compound::new();
        for section in &self.profiles {
            let marker = Marker::for_payload(self.handover, section.version, &section.payload);
            let path = marker.path(&directory, self.owner, &section.id);
            marker
                .make_durable(&path, &section.payload)
                .map_err(|error| {
                    format!(
                        "required profile {} could not be saved: {error}",
                        section.id
                    )
                })?;
            markers.insert(section.id.to_string().as_str().into(), marker.to_tag());
        }
        Ok(markers)
    }
}

/// World-local markers must never be taken from another server's player NBT.
pub(super) fn clear_markers(saved: &mut Tag) {
    if let Tag::Compound(root) = saved {
        root.remove(&MARKERS.into());
    }
}

pub(super) fn set_markers(saved: &mut Tag, markers: Compound) {
    if let Tag::Compound(root) = saved {
        root.insert(MARKERS.into(), Tag::Compound(markers));
    }
}

pub(super) fn has_markers(saved: Option<&Tag>) -> bool {
    saved.is_some_and(|saved| saved.get(MARKERS).is_some())
}

/// Read the marker for an explicitly named section. A malformed local marker
/// is an error and cannot be treated as an owner whose only copy is legacy.
fn read_marker(
    saved: Option<&Tag>,
    id: &Identifier,
) -> Result<Option<Marker>, TransferProfileReadError> {
    let Some(saved) = saved else {
        return Ok(None);
    };
    let Tag::Compound(saved) = saved else {
        return Err(TransferProfileReadError::Corrupt);
    };
    let Some(root) = saved.get(&MARKERS.into()) else {
        return Ok(None);
    };
    let Tag::Compound(root) = root else {
        return Err(TransferProfileReadError::Corrupt);
    };
    let Some(tag) = root.get(&id.to_string().as_str().into()) else {
        return Ok(None);
    };
    Marker::from_tag(tag)
        .map(Some)
        .ok_or(TransferProfileReadError::Corrupt)
}

pub(super) fn active_generation(
    world: &Path,
    owner: u128,
    saved: Option<&Tag>,
    id: &Identifier,
) -> Result<Option<u128>, TransferProfileReadError> {
    let Some(marker) = read_marker(saved, id)? else {
        return Ok(None);
    };
    let file = marker.path(&world.join("players").join("data"), owner, id);
    marker
        .verify_metadata(&file)
        .map_err(|_| TransferProfileReadError::Corrupt)?;
    Ok(Some(marker.generation))
}

pub(super) fn active_payload(
    world: &Path,
    owner: u128,
    saved: Option<&Tag>,
    id: &Identifier,
    generation: u128,
) -> Result<Vec<u8>, TransferProfileReadError> {
    let marker = read_marker(saved, id)?
        .filter(|marker| marker.generation == generation)
        .ok_or(TransferProfileReadError::StaleGeneration)?;
    let file = marker.path(&world.join("players").join("data"), owner, id);
    marker
        .verify_read(&file)
        .map_err(|_| TransferProfileReadError::Corrupt)
}

pub(super) fn verify_committed_profile(
    world: &Path,
    owner: u128,
    saved: Option<&Tag>,
    id: &Identifier,
    schema: u32,
    generation: u128,
) -> Result<(), String> {
    let marker = read_marker(saved, id)
        .ok()
        .flatten()
        .filter(|marker| marker.generation == generation && marker.schema == schema)
        .ok_or_else(|| format!("required profile {id} does not match the committed return"))?;
    let file = marker.path(&world.join("players").join("data"), owner, id);
    marker
        .verify_read(&file)
        .map(|_| ())
        .map_err(|error| format!("required profile {id} could not be verified: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_generation_is_bound_to_its_immutable_payload() {
        let id = Identifier::parse_namespaced("slimefun:profile").unwrap();
        let world = tempfile::tempdir().unwrap();
        let directory = playerdata::directory(world.path());
        let source = b"first immutable payload";
        let first = Marker::for_payload(7, 1, source);
        let path = first.path(&directory, 17, &id);
        first.make_durable(&path, source).unwrap();
        let mut player = Tag::Compound(Compound::new());
        set_markers(
            &mut player,
            [("slimefun:profile".into(), first.to_tag())].into(),
        );
        let loaded = read_marker(Some(&player), &id).unwrap().unwrap();
        assert_eq!((loaded.generation, loaded.schema), (7, 1));
        assert_eq!(loaded.verify_read(&path).unwrap(), source);

        let changed = b"other immutable payload";
        let retry = Marker::for_payload(7, 1, changed);
        assert!(retry.make_durable(&path, changed).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), source);
        std::fs::write(&path, changed).unwrap();
        assert!(loaded.verify_read(&path).is_err());
        clear_markers(&mut player);
        assert!(read_marker(Some(&player), &id).unwrap().is_none());
    }
}
