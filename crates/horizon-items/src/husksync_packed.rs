//! HuskSync v5 Bukkit snapshots. PM-Sync carries these bytes inside its SQL
//! envelope. The snapshot's own ID and metadata do not establish SQL ownership.

use std::{collections::BTreeMap, fmt, io, marker::PhantomData};

use horizon_core::{
    text::parse_java_uuid,
    version::{DATA_VERSION, VANILLA_VERSION},
};
use serde::{
    Deserialize, Serialize,
    de::{self, DeserializeOwned, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use snap::raw::{Decoder, Encoder, decompress_len};

use crate::husksync_inventory::{ENDER_CHEST_KEY, HuskPlayerItems, INVENTORY_KEY, InventoryError};

const FORMAT_VERSION: i32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BukkitMinecraftVersion {
    V26_2,
    V26_3,
}

impl BukkitMinecraftVersion {
    fn parse(value: &str) -> Result<Self, PackedError> {
        match value {
            "26.2" => Ok(Self::V26_2),
            VANILLA_VERSION => Ok(Self::V26_3),
            _ => Err(PackedError::MinecraftVersion),
        }
    }

    pub const fn data_version(self) -> i32 {
        match self {
            Self::V26_2 => 4903,
            Self::V26_3 => DATA_VERSION,
        }
    }
}

/// Use the sender's `compress_data` setting to select the adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PackedCompression {
    Json,
    /// `org.xerial.snappy.Snappy.compress`, a raw block with its size prefix.
    Snappy,
}

#[derive(Clone, Copy, Debug)]
pub struct PackedLimits {
    pub max_payload_bytes: usize,
    pub max_json_bytes: usize,
}

#[derive(Debug)]
pub struct PackedSnapshot {
    source_version: BukkitMinecraftVersion,
    metadata: BTreeMap<String, Box<RawValue>>,
    data: BTreeMap<String, String>,
}

#[derive(Debug)]
pub enum PackedError {
    SizeLimit,
    Snappy(snap::Error),
    Json(serde_json::Error),
    Metadata(&'static str),
    FormatVersion,
    Platform,
    MinecraftVersion,
    Inventory(InventoryError),
}

impl fmt::Display for PackedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeLimit => f.write_str("HuskSync Packed snapshot exceeds its size limit"),
            Self::Snappy(error) => write!(f, "invalid HuskSync Snappy data: {error}"),
            Self::Json(error) => write!(f, "invalid HuskSync JSON: {error}"),
            Self::Metadata(key) => write!(f, "invalid HuskSync snapshot field {key}"),
            Self::FormatVersion => f.write_str("expected HuskSync Packed format version 5"),
            Self::Platform => f.write_str("expected a Bukkit HuskSync snapshot"),
            Self::MinecraftVersion => {
                f.write_str("incompatible HuskSync snapshot Minecraft version")
            }
            Self::Inventory(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PackedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Snappy(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Inventory(error) => Some(error),
            _ => None,
        }
    }
}

impl PackedSnapshot {
    pub fn decode(
        bytes: &[u8],
        compression: PackedCompression,
        recipient: BukkitMinecraftVersion,
        limits: PackedLimits,
    ) -> Result<Self, PackedError> {
        if bytes.len() > java_array_limit(limits.max_payload_bytes) {
            return Err(PackedError::SizeLimit);
        }
        let raw;
        let json = match compression {
            PackedCompression::Json => bytes,
            PackedCompression::Snappy => {
                let size = decompress_len(bytes).map_err(PackedError::Snappy)?;
                if size > java_array_limit(limits.max_json_bytes) {
                    return Err(PackedError::SizeLimit);
                }
                let mut buffer = vec![0; size];
                Decoder::new()
                    .decompress(bytes, &mut buffer)
                    .map_err(PackedError::Snappy)?;
                raw = buffer;
                &raw
            }
        };
        if json.len() > java_array_limit(limits.max_json_bytes) {
            return Err(PackedError::SizeLimit);
        }

        let UniqueMap(mut metadata) = serde_json::from_slice(json).map_err(PackedError::Json)?;
        let source_version = validate_metadata(&metadata, recipient)?;
        let data = metadata
            .remove("data")
            .ok_or(PackedError::Metadata("data"))?;
        let UniqueMap(data) = serde_json::from_str(data.get()).map_err(PackedError::Json)?;
        Ok(Self {
            source_version,
            metadata,
            data,
        })
    }

    pub fn encode(
        &self,
        compression: PackedCompression,
        recipient: BukkitMinecraftVersion,
        limits: PackedLimits,
    ) -> Result<Vec<u8>, PackedError> {
        if self.source_version > recipient {
            return Err(PackedError::MinecraftVersion);
        }
        let max_payload_bytes = java_array_limit(limits.max_payload_bytes);
        let max_json_bytes = java_array_limit(limits.max_json_bytes);
        let limit = match compression {
            PackedCompression::Json => max_json_bytes.min(max_payload_bytes),
            PackedCompression::Snappy => max_json_bytes,
        };
        let mut output = BoundedJson {
            bytes: Vec::new(),
            limit,
        };
        serde_json::to_writer(&mut output, &self.document()).map_err(|error| {
            if error.is_io() {
                PackedError::SizeLimit
            } else {
                PackedError::Json(error)
            }
        })?;
        match compression {
            PackedCompression::Json => Ok(output.bytes),
            PackedCompression::Snappy => {
                // Encoder's scratch length is bounded by this already bounded JSON.
                let bytes = Encoder::new()
                    .compress_vec(&output.bytes)
                    .map_err(PackedError::Snappy)?;
                if bytes.len() > max_payload_bytes {
                    return Err(PackedError::SizeLimit);
                }
                Ok(bytes)
            }
        }
    }

    pub fn source_version(&self) -> BukkitMinecraftVersion {
        self.source_version
    }

    /// Raw JSON values retain unknown metadata without rounding numbers or
    /// changing strings. The player data map is exposed separately.
    pub fn metadata(&self) -> &BTreeMap<String, Box<RawValue>> {
        &self.metadata
    }

    pub fn data(&self) -> &BTreeMap<String, String> {
        &self.data
    }

    pub fn inventory(&self, max_field_bytes: usize) -> Result<HuskPlayerItems, InventoryError> {
        HuskPlayerItems::parse(
            self.data.get(INVENTORY_KEY).map(String::as_str),
            self.data.get(ENDER_CHEST_KEY).map(String::as_str),
            self.source_version.data_version(),
            max_field_bytes,
        )
    }

    /// Replace the two fields together. The surrounding Minecraft version
    /// must match the replacement's provenance before either field changes.
    pub fn replace_inventory(
        &mut self,
        items: &HuskPlayerItems,
        max_field_bytes: usize,
    ) -> Result<(), PackedError> {
        let version = self.source_version.data_version();
        if items.source_data_version() != version {
            return Err(PackedError::MinecraftVersion);
        }
        let fields = items
            .encode(version, max_field_bytes)
            .map_err(PackedError::Inventory)?;
        self.data.insert(INVENTORY_KEY.to_owned(), fields.inventory);
        self.data
            .insert(ENDER_CHEST_KEY.to_owned(), fields.ender_chest);
        Ok(())
    }

    fn document(&self) -> impl Serialize + '_ {
        #[derive(Serialize)]
        struct Document<'a> {
            #[serde(flatten)]
            metadata: &'a BTreeMap<String, Box<RawValue>>,
            data: &'a BTreeMap<String, String>,
        }
        Document {
            metadata: &self.metadata,
            data: &self.data,
        }
    }
}

fn java_array_limit(limit: usize) -> usize {
    limit.min(i32::MAX as usize)
}

fn validate_metadata(
    fields: &BTreeMap<String, Box<RawValue>>,
    recipient: BukkitMinecraftVersion,
) -> Result<BukkitMinecraftVersion, PackedError> {
    fn read<T: DeserializeOwned>(
        fields: &BTreeMap<String, Box<RawValue>>,
        key: &'static str,
    ) -> Result<T, PackedError> {
        fields
            .get(key)
            .and_then(|value| serde_json::from_str(value.get()).ok())
            .ok_or(PackedError::Metadata(key))
    }

    let format: i32 = read(fields, "format_version")?;
    if format != FORMAT_VERSION {
        return Err(PackedError::FormatVersion);
    }
    let platform: String = read(fields, "platform_type")?;
    if !platform.eq_ignore_ascii_case("bukkit") {
        return Err(PackedError::Platform);
    }
    let version: String = read(fields, "minecraft_version")?;
    let version = BukkitMinecraftVersion::parse(&version)?;
    if version > recipient {
        return Err(PackedError::MinecraftVersion);
    }
    let id: String = read(fields, "id")?;
    if parse_java_uuid(&id).is_none() {
        return Err(PackedError::Metadata("id"));
    }
    let _: bool = read(fields, "pinned")?;
    // Metadata time and cause remain opaque to the transfer authority.
    for key in ["timestamp", "save_cause", "server_name"] {
        let _: String = read(fields, key)?;
    }
    Ok(version)
}

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
}

impl io::Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(io::Error::other("HuskSync JSON size limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Duplicate keys must not silently replace a field before validation.
struct UniqueMap<T>(BTreeMap<String, T>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for UniqueMap<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Fields<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for Fields<T> {
            type Value = UniqueMap<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a HuskSync JSON object with unique keys")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut fields = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    if fields.insert(key, value).is_some() {
                        return Err(de::Error::custom("duplicate HuskSync JSON field"));
                    }
                }
                Ok(UniqueMap(fields))
            }
        }

        deserializer.deserialize_map(Fields(PhantomData))
    }
}
