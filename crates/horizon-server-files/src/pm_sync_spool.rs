//! The receiver's private, content-addressed transport for raw PM-Sync snapshots.
//! The caller supplies its configured world and an expected SHA-256. Ownership,
//! revision, and envelope decoding are checked by the caller at the import boundary.

use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, DirBuilder, Metadata, OpenOptions};
use sha2::{Digest, Sha256};
use std::{
    io::{self, Read, Write},
    path::Path,
};

/// Paper's default `max-snapshot-bytes`, including the envelope checksum.
pub const MAX_SNAPSHOT_BYTES: usize = 16 << 20;
const DIRECTORY: &str = "horizon-pm-sync-spool";

/// Publish exact raw snapshot bytes and return their SHA-256 handle.
pub fn write(world: &Path, bytes: &[u8]) -> io::Result<[u8; 32]> {
    check_size(bytes.len() as u64)?;
    let digest = Sha256::digest(bytes).into();
    Spool::open(world, true)?.publish(bytes, digest)?;
    Ok(digest)
}

/// Publish only the exact snapshot selected by the trusted coordinator.
pub fn write_checked(world: &Path, digest: [u8; 32], bytes: &[u8]) -> io::Result<()> {
    check_size(bytes.len() as u64)?;
    if !matches_digest(bytes, digest) {
        return Err(invalid("snapshot does not match its PM-Sync spool handle"));
    }
    Spool::open(world, true)?.publish(bytes, digest)
}

/// Read a complete exact snapshot from the world's private spool.
pub fn read(world: &Path, digest: [u8; 32]) -> io::Result<Vec<u8>> {
    let spool = Spool::open(world, false)?;
    let bytes = spool.read_entry(&entry_name(digest), digest)?;
    spool.sync_directories()?;
    Ok(bytes)
}

struct Spool {
    world: Dir,
    directory: Dir,
}

impl Spool {
    fn open(world: &Path, create: bool) -> io::Result<Self> {
        // The configured world is ambient authority. Refuse a link at its last component,
        // and use capabilities for all names beneath it so a rename cannot redirect I/O.
        let authority = cap_std::ambient_authority();
        let world = match (world.parent(), world.file_name()) {
            (Some(parent), Some(name)) => {
                let parent = if parent.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    parent
                };
                Dir::open_ambient_dir(parent, authority)?.open_dir_nofollow(name)?
            }
            _ => Dir::open_ambient_dir(world, authority)?,
        };
        let directory = match world.open_dir_nofollow(DIRECTORY) {
            Err(error) if error.kind() == io::ErrorKind::NotFound && create => {
                let mut builder = DirBuilder::new();
                #[cfg(unix)]
                cap_std::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match world.create_dir_with(DIRECTORY, &builder) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                world.open_dir_nofollow(DIRECTORY)?
            }
            other => other?,
        };
        check_private(&directory.dir_metadata()?)?;
        Ok(Self { world, directory })
    }

    fn read_entry(&self, name: &str, digest: [u8; 32]) -> io::Result<Vec<u8>> {
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        #[cfg(unix)]
        options.nonblock(true);
        let mut file = self.directory.open_with(name, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid("PM-Sync spool entry is not a regular file"));
        }
        check_private(&metadata)?;
        let len = metadata.len();
        check_size(len)?;
        let mut bytes = vec![0; len as usize];
        file.read_exact(&mut bytes)?;
        let mut extra = [0];
        if file.read(&mut extra)? != 0 || !matches_digest(&bytes, digest) {
            return Err(invalid(
                "PM-Sync spool entry has a different size or digest",
            ));
        }
        Ok(bytes)
    }

    fn publish(&self, bytes: &[u8], digest: [u8; 32]) -> io::Result<()> {
        let name = entry_name(digest);
        match self.check_retry(&name, bytes, digest) {
            Ok(()) => return self.sync_directories(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let temporary = format!(".pending-{:032x}", u128::from_be_bytes(nonce));
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        #[cfg(unix)]
        cap_std::fs::OpenOptionsExt::mode(&mut options, 0o600);
        #[cfg(windows)]
        options.sync(true);
        let mut file = self.directory.open_with(&temporary, &options)?;
        let written = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            // A link exposes the synced file atomically and cannot replace a name won by a retry.
            match self.directory.hard_link(&temporary, &self.directory, &name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            self.check_retry(&name, bytes, digest)
        })();
        let cleaned = self.directory.remove_file(&temporary);
        written.and(cleaned)?;
        self.sync_directories()
    }

    fn check_retry(&self, name: &str, bytes: &[u8], digest: [u8; 32]) -> io::Result<()> {
        if self.read_entry(name, digest)? != bytes {
            return Err(invalid(
                "PM-Sync spool handle already belongs to a different snapshot",
            ));
        }
        Ok(())
    }

    fn sync_directories(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.directory.open(".")?.sync_all()?;
            self.world.open(".")?.sync_all()?;
        }
        let current = self.world.open_dir_nofollow(DIRECTORY)?.dir_metadata()?;
        let opened = self.directory.dir_metadata()?;
        if current.dev() != opened.dev() || current.ino() != opened.ino() {
            return Err(invalid("PM-Sync spool directory was replaced"));
        }
        Ok(())
    }
}

fn entry_name(digest: [u8; 32]) -> String {
    let high = u128::from_be_bytes(digest[..16].try_into().unwrap());
    let low = u128::from_be_bytes(digest[16..].try_into().unwrap());
    format!("{high:032x}{low:032x}.pms")
}

fn check_size(len: u64) -> io::Result<()> {
    if !(1..=MAX_SNAPSHOT_BYTES as u64).contains(&len) {
        return Err(invalid(
            "PM-Sync spool entry is empty or larger than 16 MiB",
        ));
    }
    Ok(())
}

fn matches_digest(bytes: &[u8], expected: [u8; 32]) -> bool {
    <[u8; 32]>::from(Sha256::digest(bytes)) == expected
}

fn check_private(metadata: &Metadata) -> io::Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid("PM-Sync spool permissions are not private"));
        }
    }
    #[cfg(windows)]
    {
        use cap_std::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(invalid("PM-Sync spool entry is a reparse point"));
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = metadata;
    Ok(())
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pm_sync_envelope::SnapshotEnvelope;
    use std::{fs, path::PathBuf};

    struct TestWorld(PathBuf);

    impl TestWorld {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::fill(&mut nonce).unwrap();
            let unique = u128::from_be_bytes(nonce);
            let path = std::env::temp_dir().join(format!("horizon-pm-sync-{unique:032x}"));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestWorld {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn full_paper_envelope_is_published_once_and_retried_exactly() {
        let world = TestWorld::new();
        // Magic, health, two empty collection counts, Husk payload length, and checksum.
        let husk = vec![0x24; MAX_SNAPSHOT_BYTES - 56];
        let bytes = SnapshotEnvelope {
            health: 20.0,
            recipes: Default::default(),
            sections: Default::default(),
            husk: &husk,
        }
        .encode(MAX_SNAPSHOT_BYTES)
        .unwrap();
        assert_eq!(bytes.len(), MAX_SNAPSHOT_BYTES);
        let digest = Sha256::digest(&bytes).into();
        assert_eq!(
            read(&world.0, digest).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(write_checked(&world.0, [0; 32], &bytes).is_err());
        assert!(write(&world.0, &vec![0; MAX_SNAPSHOT_BYTES + 1]).is_err());
        assert!(!world.0.join(DIRECTORY).exists());
        write_checked(&world.0, digest, &bytes).unwrap();
        let directory = world.0.join(DIRECTORY);
        let file = directory.join(entry_name(digest));
        let before = fs::metadata(&file).unwrap();
        assert_eq!(write(&world.0, &bytes).unwrap(), digest);
        let after = fs::metadata(&file).unwrap();
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(before.ino(), after.ino());
            assert_eq!(after.permissions().mode() & 0o777, 0o600);
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let read_back = read(&world.0, digest).unwrap();
        assert_eq!(read_back, bytes);
        assert_eq!(
            SnapshotEnvelope::decode(&read_back, MAX_SNAPSHOT_BYTES)
                .unwrap()
                .husk,
            husk
        );
        assert_eq!(fs::read_dir(directory).unwrap().count(), 1);
        fs::OpenOptions::new()
            .write(true)
            .open(file)
            .unwrap()
            .set_len(MAX_SNAPSHOT_BYTES as u64 + 1)
            .unwrap();
        assert_eq!(
            read(&world.0, digest).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn concurrent_exact_writers_leave_one_blob_and_remove_their_temporary_files() {
        const WRITERS: usize = 6;
        let world = TestWorld::new();
        let bytes = vec![0xa5; 1 << 20];
        let digest = Sha256::digest(&bytes).into();
        let gate = std::sync::Barrier::new(WRITERS);
        std::thread::scope(|scope| {
            for _ in 0..WRITERS {
                scope.spawn(|| {
                    gate.wait();
                    write_checked(&world.0, digest, &bytes).unwrap();
                });
            }
        });
        assert_eq!(read(&world.0, digest).unwrap(), bytes);
        assert_eq!(fs::read_dir(world.0.join(DIRECTORY)).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn renamed_directory_symlinks_and_corrupt_entries_never_redirect_or_overwrite() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let world = TestWorld::new();
        let outside = TestWorld::new();
        let bytes = b"a frozen Paper snapshot";
        let digest = Sha256::digest(bytes).into();
        let outside_file = outside.0.join(entry_name(digest));
        fs::write(&outside_file, bytes).unwrap();
        fs::set_permissions(&outside_file, fs::Permissions::from_mode(0o600)).unwrap();

        let directory = world.0.join(DIRECTORY);
        symlink(&outside.0, &directory).unwrap();
        assert!(read(&world.0, digest).is_err());
        assert!(write_checked(&world.0, digest, bytes).is_err());
        fs::remove_file(&directory).unwrap();

        let spool = Spool::open(&world.0, true).unwrap();
        let moved = world.0.join("moved-spool");
        fs::rename(&directory, &moved).unwrap();
        symlink(&outside.0, &directory).unwrap();
        assert!(spool.publish(bytes, digest).is_err());
        assert_eq!(fs::read(moved.join(entry_name(digest))).unwrap(), bytes);
        assert!(read(&world.0, digest).is_err());
        assert_eq!(fs::read_dir(&outside.0).unwrap().count(), 1);
        assert_eq!(fs::read(&outside_file).unwrap(), bytes);

        let entry = moved.join(entry_name(digest));
        fs::remove_file(&entry).unwrap();
        symlink(&outside_file, &entry).unwrap();
        assert!(spool.publish(bytes, digest).is_err());
        assert!(spool.read_entry(&entry_name(digest), digest).is_err());
        assert!(
            fs::symlink_metadata(&entry)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(&entry).unwrap();

        fs::write(&entry, b"corrupt local bytes").unwrap();
        fs::set_permissions(&entry, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(spool.publish(bytes, digest).is_err());
        assert!(spool.read_entry(&entry_name(digest), digest).is_err());
        assert_eq!(fs::read(&entry).unwrap(), b"corrupt local bytes");
        assert_eq!(fs::read_dir(moved).unwrap().count(), 1);
        assert_eq!(fs::read(&outside_file).unwrap(), bytes);
    }
}
