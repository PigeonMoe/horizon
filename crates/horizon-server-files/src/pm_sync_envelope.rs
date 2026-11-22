//! PM-Sync v2 `SnapshotEnvelope`, stored in the SQL snapshot BLOB.
//! The SQL lease, owner and revision belong to the containing row. HuskSync
//! Packed data and individual plugin sections are opaque to this format.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use horizon_core::nbt::JavaString;
use sha2::{Digest, Sha256};

const MAGIC: [u8; 4] = *b"PMS\x02";
const CHECKSUM_SIZE: usize = 32;
const MAX_RECIPES: usize = 100_000;
const MAX_SECTIONS: usize = 32;

/// The original format uses Java `String` sorting and preserves UTF-16 code
/// units. Decoding borrows every plugin section and the HuskSync bytes.
#[derive(Clone, Debug)]
pub struct SnapshotEnvelope<'a> {
    pub health: f64,
    pub recipes: BTreeSet<JavaString>,
    pub sections: BTreeMap<String, &'a [u8]>,
    pub husk: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    SizeLimit,
    Truncated,
    Checksum,
    Version,
    Health,
    RecipeCount,
    ModifiedUtf,
    SectionCount,
    SectionName,
    DuplicateSection,
    SectionLength,
    HuskLength,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SizeLimit => "PM-Sync snapshot exceeds the configured limit",
            Self::Truncated => "truncated PM-Sync snapshot",
            Self::Checksum => "PM-Sync snapshot checksum mismatch",
            Self::Version => "unknown PM-Sync snapshot version",
            Self::Health => "invalid PM-Sync health",
            Self::RecipeCount => "invalid PM-Sync recipe count",
            Self::ModifiedUtf => "invalid PM-Sync Java UTF field",
            Self::SectionCount => "invalid PM-Sync section count",
            Self::SectionName => "invalid PM-Sync section name",
            Self::DuplicateSection => "duplicate PM-Sync section",
            Self::SectionLength => "invalid PM-Sync section length",
            Self::HuskLength => "invalid PM-Sync HuskSync length",
        })
    }
}

impl std::error::Error for EnvelopeError {}

impl<'a> SnapshotEnvelope<'a> {
    /// Like the Java decoder, accepts noncanonical UTF, recipe duplicates and
    /// different input order. The limit includes the 32-byte checksum.
    pub fn decode(bytes: &'a [u8], max_bytes: usize) -> Result<Self, EnvelopeError> {
        if bytes.len() > max_bytes.min(i32::MAX as usize) {
            return Err(EnvelopeError::SizeLimit);
        }
        let body_len = bytes
            .len()
            .checked_sub(CHECKSUM_SIZE)
            .ok_or(EnvelopeError::Truncated)?;
        let (body, checksum) = bytes.split_at(body_len);
        let digest: [u8; CHECKSUM_SIZE] = Sha256::digest(body).into();
        if checksum != digest {
            return Err(EnvelopeError::Checksum);
        }
        let mut reader = Reader(body);
        if reader.fixed::<4>()? != MAGIC {
            return Err(EnvelopeError::Version);
        }
        let health = f64::from_be_bytes(reader.fixed()?);
        validate_health(health)?;

        let count = reader.bounded_count(MAX_RECIPES, EnvelopeError::RecipeCount)?;
        let mut recipes = BTreeSet::new();
        for _ in 0..count {
            recipes.insert(reader.java_utf()?);
        }

        let count = reader.bounded_count(MAX_SECTIONS, EnvelopeError::SectionCount)?;
        let mut sections = BTreeMap::new();
        for _ in 0..count {
            let name = reader
                .java_utf()?
                .to_string()
                .map_err(|_| EnvelopeError::SectionName)?;
            validate_section_name(&name)?;
            let bytes = reader.payload(EnvelopeError::SectionLength)?;
            if sections.insert(name, bytes).is_some() {
                return Err(EnvelopeError::DuplicateSection);
            }
        }

        let husk = reader.payload(EnvelopeError::HuskLength)?;
        if !reader.0.is_empty() {
            return Err(EnvelopeError::HuskLength);
        }
        Ok(Self {
            health,
            recipes,
            sections,
            husk,
        })
    }

    /// Write the canonical Java encoding. Returns an error for fields the Java
    /// decoder would reject or for a snapshot larger than `max_bytes`.
    pub fn encode(&self, max_bytes: usize) -> Result<Vec<u8>, EnvelopeError> {
        validate_health(self.health)?;
        if self.recipes.len() > MAX_RECIPES {
            return Err(EnvelopeError::RecipeCount);
        }
        if self.sections.len() > MAX_SECTIONS {
            return Err(EnvelopeError::SectionCount);
        }
        let limit = max_bytes
            .min(i32::MAX as usize)
            .checked_sub(CHECKSUM_SIZE)
            .ok_or(EnvelopeError::SizeLimit)?;
        let mut writer = Writer {
            bytes: Vec::new(),
            limit,
        };
        writer.put(&MAGIC)?;
        writer.put(&self.health.to_be_bytes())?;
        writer.put(&(self.recipes.len() as i32).to_be_bytes())?;
        for recipe in &self.recipes {
            let bytes = recipe
                .to_modified_utf8()
                .ok_or(EnvelopeError::ModifiedUtf)?;
            writer.java_utf(&bytes)?;
        }
        writer.put(&(self.sections.len() as i32).to_be_bytes())?;
        // The names are ASCII, so Java UTF-16 and Rust byte order agree.
        for (name, bytes) in &self.sections {
            validate_section_name(name)?;
            writer.java_utf(name.as_bytes())?;
            writer.payload(bytes, EnvelopeError::SectionLength)?;
        }
        writer.payload(self.husk, EnvelopeError::HuskLength)?;
        let digest = Sha256::digest(&writer.bytes);
        writer.bytes.extend_from_slice(&digest);
        Ok(writer.bytes)
    }
}

fn validate_health(health: f64) -> Result<(), EnvelopeError> {
    if health.is_finite() && health >= 0.0 {
        Ok(())
    } else {
        Err(EnvelopeError::Health)
    }
}

fn validate_section_name(name: &str) -> Result<(), EnvelopeError> {
    if (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(EnvelopeError::SectionName)
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], EnvelopeError> {
        let (field, rest) = self.0.split_first_chunk().ok_or(EnvelopeError::Truncated)?;
        self.0 = rest;
        Ok(*field)
    }

    fn bounded_count(&mut self, max: usize, error: EnvelopeError) -> Result<usize, EnvelopeError> {
        let count = usize::try_from(i32::from_be_bytes(self.fixed()?)).map_err(|_| error)?;
        if count > max {
            return Err(error);
        }
        Ok(count)
    }

    fn java_utf(&mut self) -> Result<JavaString, EnvelopeError> {
        let size = usize::from(u16::from_be_bytes(self.fixed()?));
        let (field, rest) = self
            .0
            .split_at_checked(size)
            .ok_or(EnvelopeError::Truncated)?;
        self.0 = rest;
        JavaString::from_modified_utf8(field).ok_or(EnvelopeError::ModifiedUtf)
    }

    fn payload(&mut self, error: EnvelopeError) -> Result<&'a [u8], EnvelopeError> {
        let size = i32::from_be_bytes(self.fixed()?);
        if size <= 0 {
            return Err(error);
        }
        let (payload, rest) = self.0.split_at_checked(size as usize).ok_or(error)?;
        self.0 = rest;
        Ok(payload)
    }
}

struct Writer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Writer {
    fn put(&mut self, bytes: &[u8]) -> Result<(), EnvelopeError> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(EnvelopeError::SizeLimit);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    fn java_utf(&mut self, bytes: &[u8]) -> Result<(), EnvelopeError> {
        let len = u16::try_from(bytes.len()).map_err(|_| EnvelopeError::ModifiedUtf)?;
        self.put(&len.to_be_bytes())?;
        self.put(bytes)
    }

    fn payload(&mut self, bytes: &[u8], error: EnvelopeError) -> Result<(), EnvelopeError> {
        if bytes.is_empty() {
            return Err(error);
        }
        let len = i32::try_from(bytes.len()).map_err(|_| error)?;
        self.put(&len.to_be_bytes())?;
        self.put(bytes)
    }
}
