//! A return becomes evidence of local ownership only when the current player
//! file holds both its activation marker and the exact committed receipt.
//! If the connection is lost after that commit, an authenticated resume must
//! verify current ownership and load the latest local player file. Replaying
//! the incoming envelope or a historical receipt can overwrite newer play.

use super::invalid;
use horizon_core::nbt::{Limits, Tag};
use horizon_server_files::transfer_routes::valid_backend_id;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::Path,
};

pub const AUTHORITY_KEY: &str = "HorizonTransferAuthority";
const RECEIPTS: &str = "HorizonArrivalReceipts";
const MAX_MARKER_BYTES: usize = 4 << 10;

/// Positive revision from an ownership CAS, within PM-Sync's SQL BIGINT range.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct AuthorityRevision(u64);

impl TryFrom<u64> for AuthorityRevision {
    type Error = &'static str;

    fn try_from(revision: u64) -> Result<Self, Self::Error> {
        if (1..=i64::MAX as u64).contains(&revision) {
            Ok(Self(revision))
        } else {
            Err("invalid transfer authority revision")
        }
    }
}

impl From<AuthorityRevision> for u64 {
    fn from(revision: AuthorityRevision) -> Self {
        revision.0
    }
}

/// This world's last exact outbound retirement. It is independent of the
/// incoming source, since a player may return through another backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetiredAnchor {
    pub owner: u128,
    pub handover: u128,
    pub revision: AuthorityRevision,
    pub record_sha256: [u8; 32],
}

/// Exact return approved by the authenticated coordinator's ownership CAS.
/// These fields alone do not authenticate a command or activate an owner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnAuthorization {
    pub retired: RetiredAnchor,
    pub handover: u128,
    pub revision: AuthorityRevision,
    pub source: String,
    pub destination: String,
    pub record_sha256: [u8; 32],
    pub request_sha256: [u8; 32],
}

impl ReturnAuthorization {
    pub(super) fn validate(&self) -> io::Result<()> {
        if self.retired.owner == 0
            || self.retired.handover == 0
            || self.handover == 0
            || self.handover == self.retired.handover
            || self.revision <= self.retired.revision
            || self.source == self.destination
            || !valid_backend_id(&self.source)
            || !valid_backend_id(&self.destination)
        {
            return Err(invalid("invalid return authority binding"));
        }
        Ok(())
    }

    pub(super) fn validate_for(
        &self,
        backend: &str,
        retired: Option<&RetiredAnchor>,
    ) -> io::Result<()> {
        self.validate()?;
        if self.destination != backend || retired != Some(&self.retired) {
            return Err(invalid(
                "the return does not supersede this source retirement",
            ));
        }
        Ok(())
    }

    /// Encode the marker to publish in the very same native player file as
    /// the receipt and profile. Only the strict confirmed commit may use it.
    pub fn marker(&self, committed_receipt: &[u8]) -> io::Result<Tag> {
        self.validate()?;
        self.check_receipt(committed_receipt)?;
        let marker = Marker {
            version: 1,
            authorization: self.clone(),
            receipt_sha256: Sha256::digest(committed_receipt).into(),
        };
        serde_json::to_vec(&marker)
            .map(Tag::ByteArray)
            .map_err(io::Error::other)
    }

    fn check_receipt(&self, bytes: &[u8]) -> io::Result<()> {
        let receipt: CommittedReceipt =
            serde_json::from_slice(bytes).map_err(|_| invalid("invalid return arrival receipt"))?;
        let binding = receipt.binding;
        if receipt.version != 2
            || binding.owner != self.retired.owner
            || binding.handover != self.handover
            || binding.authority_revision != self.revision
            || binding.source != self.source
            || binding.destination != self.destination
            || binding.record_sha256 != self.record_sha256
            || binding.request_sha256 != self.request_sha256
        {
            return Err(invalid(
                "the committed arrival receipt differs from the return",
            ));
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    version: u8,
    authorization: ReturnAuthorization,
    receipt_sha256: [u8; 32],
}

// The stable authority fields of a terminal receipt. The game's receipt owns
// its remaining information, such as the roster and join announcement.
#[derive(Deserialize, Serialize)]
struct CommittedReceipt {
    version: u8,
    binding: ReceiptBinding,
}

#[derive(Deserialize, Serialize)]
struct ReceiptBinding {
    owner: u128,
    handover: u128,
    authority_revision: AuthorityRevision,
    source: String,
    destination: String,
    record_sha256: [u8; 32],
    request_sha256: [u8; 32],
}

/// Evidence read from the current canonical player file. An older backup or
/// a staged in-memory guest can never construct this witness.
#[derive(Debug, Eq, PartialEq)]
pub struct CurrentReturn {
    authorization: ReturnAuthorization,
}

impl CurrentReturn {
    pub fn authorization(&self) -> &ReturnAuthorization {
        &self.authorization
    }

    /// No `dat_old` fallback. The caller must own the world session and
    /// drain its player writers before using the result to reconcile authority.
    pub fn read(world: &Path, owner: u128) -> io::Result<Option<Self>> {
        Self::read_player(world, owner).map(|read| read.map(|(current, _)| current))
    }

    /// Read the current player only when it proves this exact authorization.
    pub fn read_matching_player(
        world: &Path,
        expected: &ReturnAuthorization,
    ) -> io::Result<Option<(Self, Tag)>> {
        Self::read_player(world, expected.retired.owner)
            .map(|read| read.filter(|(current, _)| current.authorization() == expected))
    }

    /// The same checked file and its player data for an interrupted confirmation.
    pub fn read_player(world: &Path, owner: u128) -> io::Result<Option<(Self, Tag)>> {
        let file = crate::lan::playerdata::path(world, owner);
        for directory in [world.join("players"), world.join("players/data")] {
            match fs::symlink_metadata(directory) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => return Err(invalid("invalid return authority player directory")),
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(error),
            }
        }
        let max = Limits::saved().max_bytes as u64;
        match fs::symlink_metadata(&file) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= max => {}
            Ok(_) => return Err(invalid("invalid return authority player file")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        let file = fs::File::open(file)?;
        let mut bytes = Vec::new();
        (&file).take(max + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > max {
            return Err(invalid("return authority player file is too large"));
        }
        let tag = crate::save::decode_dat(bytes)
            .map_err(|_| invalid("return authority player file is corrupt"))?;
        let Tag::Compound(root) = &tag else {
            return Err(invalid(
                "return authority player file is not a player compound",
            ));
        };
        let Some(marker) = root.get(&AUTHORITY_KEY.into()) else {
            return Ok(None);
        };
        let Tag::ByteArray(bytes) = marker else {
            return Err(invalid("invalid return authority marker"));
        };
        if bytes.len() > MAX_MARKER_BYTES {
            return Err(invalid("return authority marker is too large"));
        }
        let marker: Marker = serde_json::from_slice(bytes)
            .map_err(|_| invalid("invalid return authority marker"))?;
        let authorization = marker.authorization;
        authorization.validate()?;
        if marker.version != 1
            || owner != authorization.retired.owner
            || !matches!(root.get(&"UUID".into()), Some(Tag::IntArray(id)) if id.as_slice() == crate::lan::playerdata::uuid_ints(owner))
        {
            return Err(invalid("return authority marker belongs to another player"));
        }
        let key = format!("{:032x}", authorization.handover);
        let Some(Tag::ByteArray(receipt)) =
            root.get(&RECEIPTS.into()).and_then(|tag| tag.get(&key))
        else {
            return Err(invalid("return authority has no committed receipt"));
        };
        if Sha256::digest(receipt).as_slice() != marker.receipt_sha256 {
            return Err(invalid("return authority receipt digest differs"));
        }
        authorization.check_receipt(receipt)?;
        #[cfg(unix)]
        {
            file.sync_all()?;
            fs::File::open(world.join("players/data"))?.sync_all()?;
        }
        Ok(Some((Self { authorization }, tag)))
    }
}

#[cfg(test)]
pub(super) fn committed_player(authorization: &ReturnAuthorization) -> Tag {
    let owner = authorization.retired.owner;
    let receipt = serde_json::to_vec(&CommittedReceipt {
        version: 2,
        binding: ReceiptBinding {
            owner,
            handover: authorization.handover,
            authority_revision: authorization.revision,
            source: authorization.source.clone(),
            destination: authorization.destination.clone(),
            record_sha256: authorization.record_sha256,
            request_sha256: authorization.request_sha256,
        },
    })
    .unwrap();
    let marker = authorization.marker(&receipt).unwrap();
    Tag::compound([
        (
            "UUID",
            Tag::IntArray(crate::lan::playerdata::uuid_ints(owner).to_vec()),
        ),
        (AUTHORITY_KEY, marker),
        (
            RECEIPTS,
            Tag::compound([(
                format!("{:032x}", authorization.handover).as_str(),
                Tag::ByteArray(receipt),
            )]),
        ),
    ])
}
