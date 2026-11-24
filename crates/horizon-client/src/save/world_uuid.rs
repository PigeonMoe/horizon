//! Paper and Horizon dimension identity, independent of vanilla save metadata.

use flate2::read::GzDecoder;
use horizon_core::nbt::{self, Limits, Tag};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

const PAPER_METADATA: &str = "data/paper/metadata.dat";
const HORIZON_UUID: &str = "data/horizon/world_uuid.dat";
const PAPER_METADATA_LIMIT: usize = 1024 * 1024;
const UUID_BYTES: usize = 16;

pub(super) fn load_or_create(dimension: &Path, legacy_root: Option<&Path>) -> crate::Result<u128> {
    if let Some(uuid) = read_paper(&dimension.join(PAPER_METADATA))? {
        return Ok(uuid);
    }
    if let Some(uuid) = read_binary(&dimension.join("uid.dat"))? {
        return Ok(uuid);
    }
    if let Some(root) = legacy_root
        && let Some(uuid) = read_binary(&root.join("uid.dat"))?
    {
        return Ok(uuid);
    }

    let path = dimension.join(HORIZON_UUID);
    if let Some(uuid) = read_binary(&path)? {
        return Ok(uuid);
    }
    let mut bytes = [0; UUID_BYTES];
    getrandom::fill(&mut bytes).map_err(|why| io::Error::other(why.to_string()))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    super::write_durable(&path, &bytes)?;
    Ok(u128::from_be_bytes(bytes))
}

fn read_paper(path: &Path) -> io::Result<Option<u128>> {
    let Some(mut bytes) = read_optional(path, PAPER_METADATA_LIMIT)? else {
        return Ok(None);
    };
    if bytes.starts_with(&[0x1f, 0x8b]) {
        bytes = read_bounded(GzDecoder::new(bytes.as_slice()), PAPER_METADATA_LIMIT)?;
    }
    let (root, consumed) = nbt::read_named(
        &bytes,
        Limits {
            max_bytes: PAPER_METADATA_LIMIT,
            max_nodes: 4096,
            max_depth: 32,
        },
    )
    .map_err(|why| io::Error::new(io::ErrorKind::InvalidData, why))?;
    if consumed != bytes.len() {
        return Err(invalid("extra bytes in Paper metadata.dat"));
    }
    let Some(uuid) = root.tag.get("data").and_then(|data| data.get("uuid")) else {
        return Ok(None);
    };
    let Tag::IntArray(words) = uuid else {
        return Err(invalid("Paper data.uuid is not an NBT int array"));
    };
    let [first, second, third, fourth] = words.as_slice() else {
        return Err(invalid("Paper data.uuid must contain four ints"));
    };
    let uuid = [first, second, third, fourth]
        .into_iter()
        .fold(0u128, |bits, word| (bits << 32) | u128::from(*word as u32));
    Ok(Some(uuid))
}

fn read_binary(path: &Path) -> io::Result<Option<u128>> {
    let Some(bytes) = read_optional(path, UUID_BYTES)? else {
        return Ok(None);
    };
    let bytes = bytes
        .as_slice()
        .try_into()
        .map_err(|_| invalid("world UUID must contain 16 bytes"))?;
    Ok(Some(u128::from_be_bytes(bytes)))
}

fn read_optional(path: &Path, limit: usize) -> io::Result<Option<Vec<u8>>> {
    match File::open(path) {
        Ok(file) => read_bounded(file, limit).map(Some),
        Err(why) if why.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(why) => Err(why),
    }
}

fn read_bounded(source: impl Read, limit: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    source.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(invalid("world UUID data exceeds its size limit"));
    }
    Ok(bytes)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::{fs, io::Write};

    #[test]
    fn native_dimensions_have_distinct_uuids_after_cold_reopen() {
        let world = tempfile::tempdir().unwrap();
        let overworld = world.path().join("dimensions/minecraft/overworld");
        let nether = world.path().join("dimensions/minecraft/the_nether");
        let overworld_uuid = load_or_create(&overworld, Some(world.path())).unwrap();
        let nether_uuid = load_or_create(&nether, None).unwrap();

        assert_ne!(overworld_uuid, nether_uuid);
        for (folder, expected) in [(&overworld, overworld_uuid), (&nether, nether_uuid)] {
            let bytes = fs::read(folder.join(HORIZON_UUID)).unwrap();
            assert_eq!(bytes, expected.to_be_bytes());
            assert_eq!(bytes[6] >> 4, 4);
            assert_eq!(bytes[8] >> 6, 2);
            assert_eq!(load_or_create(folder, None).unwrap(), expected);
        }
    }

    #[test]
    fn migrated_paper_uuid_preserves_all_bits_and_takes_precedence() {
        let world = tempfile::tempdir().unwrap();
        let root = world.path().join("dimensions/minecraft/overworld");
        let native_uuid = load_or_create(&root, Some(world.path())).unwrap();
        let path = root.join(PAPER_METADATA);
        let expected = 0x752d5d2e_80e8_4d64_bc4d_4c60790f6bd4u128;
        let metadata = nbt::write_named(
            &nbt::NamedTag {
                name: "".into(),
                tag: Tag::compound([
                    ("DataVersion", Tag::Int(4830)),
                    (
                        "data",
                        Tag::compound([(
                            "uuid",
                            Tag::IntArray(
                                expected
                                    .to_be_bytes()
                                    .chunks_exact(4)
                                    .map(|word| i32::from_be_bytes(word.try_into().unwrap()))
                                    .collect(),
                            ),
                        )]),
                    ),
                ]),
            },
            Limits::default(),
        )
        .unwrap();
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&metadata).unwrap();
        let compressed = gzip.finish().unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, &compressed).unwrap();

        assert_eq!(load_or_create(&root, Some(world.path())).unwrap(), expected);
        assert_eq!(load_or_create(&root, None).unwrap(), expected);
        assert_eq!(fs::read(&path).unwrap(), compressed);
        assert_eq!(
            fs::read(root.join(HORIZON_UUID)).unwrap(),
            native_uuid.to_be_bytes()
        );
    }

    #[test]
    fn legacy_paper_uid_is_used_only_for_its_own_dimension() {
        let world = tempfile::tempdir().unwrap();
        let overworld = world.path().join("dimensions/minecraft/overworld");
        let nether = world.path().join("dimensions/minecraft/the_nether");
        let overworld_uuid = 0xfedcba9876543210_0123456789abcdefu128;
        let nether_uuid = 0x1234567890abcdef_fedcba9876543210u128;
        fs::write(world.path().join("uid.dat"), overworld_uuid.to_be_bytes()).unwrap();
        fs::create_dir_all(&nether).unwrap();
        fs::write(nether.join("uid.dat"), nether_uuid.to_be_bytes()).unwrap();

        assert_eq!(
            load_or_create(&overworld, Some(world.path())).unwrap(),
            overworld_uuid
        );
        assert_eq!(load_or_create(&nether, None).unwrap(), nether_uuid);
        assert!(!overworld.join(HORIZON_UUID).exists());
        assert!(!nether.join(HORIZON_UUID).exists());
    }

    #[test]
    fn oversized_paper_metadata_does_not_create_a_replacement_uuid() {
        let world = tempfile::tempdir().unwrap();
        let root = world.path();
        let path = root.join(PAPER_METADATA);
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(&vec![0; PAPER_METADATA_LIMIT + 1]).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, gzip.finish().unwrap()).unwrap();

        assert!(load_or_create(root, None).is_err());
        assert!(!root.join(HORIZON_UUID).exists());
    }
}
