//! Durable source ownership for strict hand-overs. A published record fences
//! its owner even after the connection or process has gone away.

mod authority;
pub use authority::{
    AUTHORITY_KEY, AuthorityRevision, CurrentReturn, RetiredAnchor, ReturnAuthorization,
};

use horizon_server_files::{
    durable,
    transfer_routes::{BACKEND_ID_MAX_BYTES, valid_backend_id},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::Mutex,
};

const DIRECTORY: &str = "horizon-source-handovers";
const EXTENSION: &str = "handover";
const LEGACY_MAGIC: &[u8; 4] = b"HSH1";
const VERSIONED_MAGIC: &[u8; 4] = b"HSH2";
const MAX_PAYLOAD: usize = 32 << 20;
const HEADER: usize = 4 + 1 + 16 + 16 + 1 + 4 + 4;
const REVISION_BYTES: usize = std::mem::size_of::<u64>();
const MAX_FILE: usize = HEADER + REVISION_BYTES + BACKEND_ID_MAX_BYTES + 2 * MAX_PAYLOAD + 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    Pending = 1,
    Retired = 2,
}

/// The portable envelope is sent only to the selected destination. The
/// checkpoint keeps the source's own fields for recovery on cancellation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Handover {
    pub owner: u128,
    pub id: u128,
    pub destination: String,
    pub record: Vec<u8>,
    pub local_checkpoint: Vec<u8>,
}

impl Handover {
    pub fn record_digest(&self) -> [u8; 32] {
        Sha256::digest(&self.record).into()
    }

    fn encode(&self, phase: Phase, revision: Option<AuthorityRevision>) -> io::Result<Vec<u8>> {
        validate_fields(
            self.owner,
            self.id,
            &self.destination,
            self.record.len(),
            self.local_checkpoint.len(),
        )?;
        let mut bytes = Vec::with_capacity(
            HEADER
                + REVISION_BYTES
                + self.destination.len()
                + self.record.len()
                + self.local_checkpoint.len()
                + 32,
        );
        bytes.extend_from_slice(if revision.is_some() {
            VERSIONED_MAGIC
        } else {
            LEGACY_MAGIC
        });
        bytes.push(phase as u8);
        bytes.extend_from_slice(&self.owner.to_be_bytes());
        bytes.extend_from_slice(&self.id.to_be_bytes());
        bytes.push(self.destination.len() as u8);
        bytes.extend_from_slice(&(self.record.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&(self.local_checkpoint.len() as u32).to_be_bytes());
        if let Some(revision) = revision {
            bytes.extend_from_slice(&u64::from(revision).to_be_bytes());
        }
        bytes.extend_from_slice(self.destination.as_bytes());
        bytes.extend_from_slice(&self.record);
        bytes.extend_from_slice(&self.local_checkpoint);
        let checksum = Sha256::digest(&bytes);
        bytes.extend_from_slice(&checksum);
        Ok(bytes)
    }
}

/// A compact read-side binding. Payloads are read only when a coordinator
/// explicitly needs to recover a hand-over.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Fence {
    id: u128,
    seal: [u8; 32],
}

#[derive(Debug)]
struct Reservation {
    fence: Fence,
    phase: Option<Phase>,
    revision: Option<AuthorityRevision>,
    /// Keep the original capture only until initial publication succeeds.
    unpublished: Option<Handover>,
}

#[derive(Debug)]
pub struct Journal {
    directory: PathBuf,
    // Serialize transitions and publication with the login/admission readers.
    fences: Mutex<HashMap<u128, Reservation>>,
}

impl Journal {
    /// Called after the server opens the world, before it accepts logins. A
    /// missing journal is normal, even on worlds using ordinary transfers.
    pub fn open(world: &Path) -> io::Result<Self> {
        let directory = world.join(DIRECTORY);
        let mut fences = HashMap::new();
        match fs::symlink_metadata(&directory) {
            Ok(metadata) if !metadata.is_dir() => {
                return Err(invalid("source hand-over journal is not a directory"));
            }
            Ok(_) => {
                for entry in fs::read_dir(&directory)? {
                    let path = entry?.path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some(EXTENSION) {
                        let owner = path
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .and_then(|stem| {
                                u128::from_str_radix(stem, 16)
                                    .ok()
                                    .filter(|owner| format!("{owner:032x}") == stem)
                            })
                            .ok_or_else(|| invalid("invalid source hand-over filename"))?;
                        let Entry {
                            phase,
                            handover,
                            fence,
                            revision,
                        } = read_entry(&path)?;
                        if handover.owner != owner {
                            return Err(invalid(
                                "source hand-over owner does not match its filename",
                            ));
                        }
                        fences.insert(
                            owner,
                            Reservation {
                                fence,
                                phase: Some(phase),
                                revision,
                                unpublished: None,
                            },
                        );
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        Ok(Self {
            directory,
            fences: Mutex::new(fences),
        })
    }

    pub fn fenced_handover(&self, owner: u128) -> Option<u128> {
        self.fences
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&owner)
            .map(|entry| entry.fence.id)
    }

    /// Read a retired ID without opening a payload again. Control replies
    /// first call `retire` to durably reconcile the exact destination and digest.
    pub fn retired_handover(&self, owner: u128) -> Option<u128> {
        let fences = self
            .fences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let active = fences.get(&owner)?;
        (active.phase == Some(Phase::Retired)).then_some(active.fence.id)
    }

    /// The exact local retirement a returning owner must supersede. HSH1
    /// has no ownership revision, so its fence cannot issue this anchor.
    pub fn retired_anchor(&self, owner: u128) -> io::Result<Option<RetiredAnchor>> {
        let fences = self
            .fences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(active) = fences
            .get(&owner)
            .filter(|entry| entry.phase == Some(Phase::Retired))
        else {
            return Ok(None);
        };
        let Some(revision) = active.revision else {
            return Ok(None);
        };
        let entry = read_entry(&self.path(owner))?;
        if entry.phase != Phase::Retired
            || entry.revision != Some(revision)
            || entry.fence != active.fence
            || entry.handover.owner != owner
        {
            return Err(invalid(
                "the durable source retirement differs from its fence",
            ));
        }
        Ok(Some(RetiredAnchor {
            owner,
            handover: entry.handover.id,
            revision,
            record_sha256: entry.handover.record_digest(),
        }))
    }

    /// Check an already authenticated return against this world's old
    /// retirement. This authorizes staging only and leaves the fence in place.
    pub fn validate_return(
        &self,
        backend: &str,
        authorization: &ReturnAuthorization,
    ) -> io::Result<()> {
        let retired = self.retired_anchor(authorization.retired.owner)?;
        authorization.validate_for(backend, retired.as_ref())
    }

    /// Read the exact current-file witness for reconciliation. An older
    /// backup, an uncommitted arrival or an HSH1 record cannot provide it.
    /// The gameplay authority remains fenced until the caller completes the
    /// coordinated activation.
    pub fn current_return(&self, backend: &str, owner: u128) -> io::Result<Option<CurrentReturn>> {
        let Some(retired) = self.retired_anchor(owner)? else {
            return Ok(None);
        };
        let world = self.directory.parent().expect("world path");
        let Some(current) = CurrentReturn::read(world, owner)? else {
            return Ok(None);
        };
        current
            .authorization()
            .validate_for(backend, Some(&retired))?;
        Ok(Some(current))
    }

    /// Publish a new frozen hand-over, or acknowledge the same immutable
    /// hand-over. The caller must first settle all mutable state on the game
    /// thread; this does not park a player or perform plugin capture.
    pub fn prepare(&self, handover: &Handover) -> io::Result<Phase> {
        self.prepare_at(handover, None)
    }

    /// Prepare a new hand-over bound to the coordinator's ownership CAS
    /// revision. The retirement must acknowledge the same sealed snapshot.
    pub fn prepare_versioned(
        &self,
        handover: &Handover,
        revision: AuthorityRevision,
    ) -> io::Result<Phase> {
        self.prepare_at(handover, Some(revision))
    }

    fn prepare_at(
        &self,
        handover: &Handover,
        revision: Option<AuthorityRevision>,
    ) -> io::Result<Phase> {
        let bytes = handover.encode(Phase::Pending, revision)?;
        let next = fence(handover.id, &bytes);
        let mut fences = self
            .fences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if fences
            .get(&handover.owner)
            .is_some_and(|prior| prior.fence != next)
        {
            return Err(invalid("another source hand-over already owns this player"));
        }
        if !fences.contains_key(&handover.owner)
            && fences.values().any(|prior| prior.unpublished.is_some())
        {
            return Err(io::Error::other(
                "an unfinished source publication must be recovered first",
            ));
        }
        let path = self.path(handover.owner);
        let phase = match read_entry(&path) {
            Ok(previous) => {
                if &previous.handover != handover || previous.revision != revision {
                    return Err(invalid(
                        "the source hand-over differs from the prepared record",
                    ));
                }
                previous.phase
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Phase::Pending,
            Err(error) => return Err(error),
        };
        // A durable writer can error after rename. Keep the owner fenced
        // until this exact request can be reconciled, whether or not the file
        // is immediately visible. Restart uses the published file alone.
        let active = fences.entry(handover.owner).or_insert(Reservation {
            fence: next,
            phase: None,
            revision,
            unpublished: None,
        });
        let bytes = if phase == Phase::Pending {
            bytes
        } else {
            handover.encode(phase, revision)?
        };
        if let Err(error) = self.publish(&path, &bytes) {
            if active.phase.is_none() && active.unpublished.is_none() {
                active.unpublished = Some(handover.clone());
            }
            return Err(error);
        }
        active.phase = Some(phase);
        active.unpublished = None;
        Ok(phase)
    }

    /// Recover the immutable snapshot only after rechecking its on-disk seal
    /// and republishing the exact entry if an earlier reply was ambiguous.
    pub fn recover(&self, owner: u128, id: u128) -> io::Result<(Phase, Handover)> {
        let mut fences = self
            .fences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let path = self.path(owner);
        let active = fences
            .get_mut(&owner)
            .filter(|entry| entry.fence.id == id)
            .ok_or_else(|| invalid("the source hand-over does not match"))?;
        let (phase, handover) = match read_entry(&path) {
            Ok(entry)
                if entry.handover.owner == owner
                    && entry.fence == active.fence
                    && entry.revision == active.revision =>
            {
                (entry.phase, entry.handover)
            }
            Ok(_) => return Err(invalid("the source hand-over does not match")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let handover = active.unpublished.clone().ok_or(error)?;
                (Phase::Pending, handover)
            }
            Err(error) => return Err(error),
        };
        self.publish(&path, &handover.encode(phase, active.revision)?)?;
        active.phase = Some(phase);
        active.unpublished = None;
        Ok((phase, handover))
    }

    /// Give up source authority for exactly this destination and snapshot.
    /// The retained entry is also the idempotent receipt after disconnection.
    pub fn retire(
        &self,
        owner: u128,
        id: u128,
        destination: &str,
        record_digest: [u8; 32],
    ) -> io::Result<()> {
        let mut fences = self
            .fences
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let path = self.path(owner);
        let Entry {
            handover,
            fence,
            revision,
            ..
        } = read_entry(&path)?;
        let active = fences
            .get_mut(&owner)
            .filter(|entry| entry.fence == fence && entry.revision == revision)
            .ok_or_else(|| invalid("the source hand-over does not match"))?;
        if handover.owner != owner
            || handover.id != id
            || handover.destination != destination
            || handover.record_digest() != record_digest
        {
            return Err(invalid("the source hand-over does not match"));
        }
        self.publish(&path, &handover.encode(Phase::Retired, revision)?)?;
        active.phase = Some(Phase::Retired);
        active.unpublished = None;
        Ok(())
    }

    fn path(&self, owner: u128) -> PathBuf {
        self.directory.join(format!("{owner:032x}.{EXTENSION}"))
    }

    fn publish(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        match fs::create_dir(&self.directory) {
            Ok(()) => {}
            Err(error)
                if error.kind() == io::ErrorKind::AlreadyExists
                    && fs::symlink_metadata(&self.directory)?.is_dir() => {}
            Err(error) => return Err(error),
        }
        // The first record's directory entry must also survive power loss.
        #[cfg(unix)]
        fs::File::open(self.directory.parent().expect("world path"))?.sync_all()?;
        durable::write(path, bytes)
    }
}

fn validate_fields(
    owner: u128,
    id: u128,
    destination: &str,
    record_len: usize,
    checkpoint_len: usize,
) -> io::Result<()> {
    if owner == 0
        || id == 0
        || !valid_backend_id(destination)
        || !(1..=MAX_PAYLOAD).contains(&record_len)
        || !(1..=MAX_PAYLOAD).contains(&checkpoint_len)
    {
        return Err(invalid("invalid source hand-over fields"));
    }
    Ok(())
}

struct Entry {
    phase: Phase,
    handover: Handover,
    fence: Fence,
    revision: Option<AuthorityRevision>,
}

fn read_entry(path: &Path) -> io::Result<Entry> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.len() > MAX_FILE as u64 {
        return Err(invalid("invalid source hand-over file"));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)?;
    let malformed = || invalid("malformed source hand-over record");
    let (body, seal) = bytes
        .split_at_checked(bytes.len().checked_sub(32).ok_or_else(malformed)?)
        .ok_or_else(malformed)?;
    if body.len() < HEADER || bytes.len() > MAX_FILE || Sha256::digest(body).as_slice() != seal {
        return Err(malformed());
    }
    let phase = match body[4] {
        1 => Phase::Pending,
        2 => Phase::Retired,
        _ => return Err(malformed()),
    };
    let owner = u128::from_be_bytes(body[5..21].try_into().unwrap());
    let id = u128::from_be_bytes(body[21..37].try_into().unwrap());
    let destination_len = usize::from(body[37]);
    let record_len = u32::from_be_bytes(body[38..42].try_into().unwrap()) as usize;
    let checkpoint_len = u32::from_be_bytes(body[42..46].try_into().unwrap()) as usize;
    let (revision, contents) = match &body[..LEGACY_MAGIC.len()] {
        magic if magic == LEGACY_MAGIC => (None, &body[HEADER..]),
        magic if magic == VERSIONED_MAGIC => {
            let (revision, contents) = body[HEADER..]
                .split_at_checked(REVISION_BYTES)
                .ok_or_else(malformed)?;
            let revision =
                AuthorityRevision::try_from(u64::from_be_bytes(revision.try_into().unwrap()))
                    .map_err(invalid)?;
            (Some(revision), contents)
        }
        _ => return Err(malformed()),
    };
    let (destination, payload) = contents
        .split_at_checked(destination_len)
        .ok_or_else(malformed)?;
    let destination = std::str::from_utf8(destination).map_err(|_| malformed())?;
    validate_fields(owner, id, destination, record_len, checkpoint_len)?;
    let (record, checkpoint) = payload.split_at_checked(record_len).ok_or_else(malformed)?;
    if checkpoint.len() != checkpoint_len {
        return Err(malformed());
    }
    Ok(Entry {
        phase,
        handover: Handover {
            owner,
            id,
            destination: destination.into(),
            record: record.to_vec(),
            local_checkpoint: checkpoint.to_vec(),
        },
        fence: fence(id, &bytes),
        revision,
    })
}

fn fence(id: u128, bytes: &[u8]) -> Fence {
    // Phase can change but the immutable identity of the source record cannot.
    let seal = Sha256::digest(&bytes[5..bytes.len() - 32]).into();
    Fence { id, seal }
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason)
}

pub(crate) fn refusal() -> horizon_core::nbt::Tag {
    use horizon_core::nbt::Tag;
    use horizon_protocol::text::literal;
    Tag::compound([
        ("translate", literal("horizon.transfer.source_fenced")),
        (
            "fallback",
            literal("Your player data is reserved for a transfer to another server."),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handover() -> Handover {
        Handover {
            owner: 124,
            id: 456,
            destination: "destination-two".into(),
            record: b"portable inventory and named profiles".to_vec(),
            local_checkpoint: b"local position, spawn and player data".to_vec(),
        }
    }

    fn a_return(transfer: &Handover) -> ReturnAuthorization {
        ReturnAuthorization {
            retired: RetiredAnchor {
                owner: transfer.owner,
                handover: transfer.id,
                revision: AuthorityRevision::try_from(17).unwrap(),
                record_sha256: transfer.record_digest(),
            },
            handover: 789,
            revision: AuthorityRevision::try_from(23).unwrap(),
            source: "another-hop".into(),
            destination: "original-backend".into(),
            record_sha256: [9; 32],
            request_sha256: [7; 32],
        }
    }

    #[test]
    fn source_handover_requires_exact_retirement_and_a_newer_current_return() {
        let world = tempfile::tempdir().unwrap();
        let handover = handover();
        let grant = a_return(&handover);
        let owner = handover.owner;
        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(
            journal
                .prepare_versioned(&handover, grant.retired.revision)
                .unwrap(),
            Phase::Pending
        );
        assert!(journal.prepare(&handover).is_err());

        let file = crate::lan::playerdata::path(world.path(), owner);
        let profile = authority::committed_player(&grant);
        crate::save::write_dat_durable(&file, profile.clone()).unwrap();
        assert!(journal.validate_return(&grant.destination, &grant).is_err());
        assert!(
            journal
                .current_return(&grant.destination, owner)
                .unwrap()
                .is_none()
        );
        journal
            .retire(
                owner,
                handover.id,
                &handover.destination,
                handover.record_digest(),
            )
            .unwrap();
        assert_eq!(
            journal.retired_anchor(owner).unwrap(),
            Some(grant.retired.clone())
        );
        assert!(journal.validate_return(&grant.destination, &grant).is_ok());
        assert!(journal.validate_return("wrong-backend", &grant).is_err());
        let mut stale = grant.clone();
        stale.revision = grant.retired.revision;
        assert!(journal.validate_return(&grant.destination, &stale).is_err());
        let mut different_checkpoint = grant.clone();
        different_checkpoint.retired.record_sha256[0] ^= 1;
        assert!(
            journal
                .validate_return(&grant.destination, &different_checkpoint)
                .is_err()
        );
        drop(journal);

        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(
            journal.recover(owner, handover.id).unwrap(),
            (Phase::Retired, handover.clone())
        );
        let current = journal
            .current_return(&grant.destination, owner)
            .unwrap()
            .unwrap();
        assert_eq!(current.authorization(), &grant);
        assert_eq!(journal.fenced_handover(owner), Some(handover.id));

        let backup = file.with_extension("dat_old");
        fs::rename(&file, &backup).unwrap();
        assert!(
            journal
                .current_return(&grant.destination, owner)
                .unwrap()
                .is_none()
        );
        fs::write(&file, b"malformed primary").unwrap();
        assert!(journal.current_return(&grant.destination, owner).is_err());

        let mut changed_receipt = profile.clone();
        let key = format!("{:032x}", grant.handover);
        let Some(horizon_core::nbt::Tag::ByteArray(bytes)) = changed_receipt
            .get_mut("HorizonArrivalReceipts")
            .and_then(|entries| entries.get_mut(&key))
        else {
            unreachable!()
        };
        bytes[0] ^= 1;
        crate::save::write_dat_durable(&file, changed_receipt).unwrap();
        assert_eq!(
            journal
                .current_return(&grant.destination, owner)
                .unwrap_err()
                .to_string(),
            "return authority receipt digest differs"
        );

        let mut changed_owner = profile.clone();
        *changed_owner.get_mut("UUID").unwrap() =
            horizon_core::nbt::Tag::IntArray(crate::lan::playerdata::uuid_ints(owner + 1).to_vec());
        crate::save::write_dat_durable(&file, changed_owner).unwrap();
        assert!(journal.current_return(&grant.destination, owner).is_err());

        let mut profile = profile;
        let horizon_core::nbt::Tag::Compound(root) = &mut profile else {
            unreachable!()
        };
        root.remove(&"HorizonArrivalReceipts".into());
        crate::save::write_dat_durable(&file, profile).unwrap();
        assert!(journal.current_return(&grant.destination, owner).is_err());
        assert_eq!(journal.fenced_handover(owner), Some(handover.id));
    }

    #[test]
    fn source_handover_legacy_retirement_cannot_authorize_a_return() {
        let world = tempfile::tempdir().unwrap();
        let handover = handover();
        let grant = a_return(&handover);
        let owner = handover.owner;
        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(journal.prepare(&handover).unwrap(), Phase::Pending);
        journal
            .retire(
                owner,
                handover.id,
                &handover.destination,
                handover.record_digest(),
            )
            .unwrap();
        crate::save::write_dat_durable(
            &crate::lan::playerdata::path(world.path(), owner),
            authority::committed_player(&grant),
        )
        .unwrap();
        drop(journal);

        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(journal.retired_handover(owner), Some(handover.id));
        assert!(journal.retired_anchor(owner).unwrap().is_none());
        assert!(journal.validate_return(&grant.destination, &grant).is_err());
        assert!(
            journal
                .current_return(&grant.destination, owner)
                .unwrap()
                .is_none()
        );
        assert_eq!(journal.fenced_handover(owner), Some(handover.id));
    }

    #[test]
    fn source_handover_survives_restart_and_retirement_is_exact() {
        let world = tempfile::tempdir().unwrap();
        let mut transfer = handover();
        let owner = transfer.owner;
        let id = transfer.id;
        let digest = transfer.record_digest();
        let journal = Journal::open(world.path()).unwrap();
        assert!(!world.path().join(DIRECTORY).exists());
        assert_eq!(journal.fenced_handover(owner), None);
        assert_eq!(journal.prepare(&transfer).unwrap(), Phase::Pending);
        drop(journal);

        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(journal.fenced_handover(owner), Some(id));
        assert_eq!(journal.fenced_handover(owner + 1), None);
        let (phase, saved) = journal.recover(owner, id).unwrap();
        assert_eq!(phase, Phase::Pending);
        assert_eq!(saved, transfer);
        for (other, route, hash) in [
            (id + 1, transfer.destination.as_str(), digest),
            (id, "elsewhere", digest),
            (id, transfer.destination.as_str(), [42; 32]),
        ] {
            assert!(journal.retire(owner, other, route, hash).is_err());
        }
        assert_eq!(journal.recover(owner, id).unwrap().0, Phase::Pending);
        journal
            .retire(owner, id, &transfer.destination, digest)
            .unwrap();
        let retired = fs::read(journal.path(owner)).unwrap();
        drop(journal);

        let journal = Journal::open(world.path()).unwrap();
        assert_eq!(journal.fenced_handover(owner), Some(id));
        assert_eq!(journal.prepare(&transfer).unwrap(), Phase::Retired);
        journal
            .retire(owner, id, &transfer.destination, digest)
            .unwrap();
        assert_eq!(fs::read(journal.path(owner)).unwrap(), retired);
        transfer.local_checkpoint.push(0);
        assert!(journal.prepare(&transfer).is_err());
        assert_eq!(journal.recover(owner, id).unwrap(), (Phase::Retired, saved));

        let mut corrupted = retired;
        corrupted[HEADER] ^= 1;
        fs::write(journal.path(owner), corrupted).unwrap();
        assert!(journal.recover(owner, id).is_err());
        assert_eq!(journal.fenced_handover(owner), Some(id));
        assert_eq!(
            Journal::open(world.path()).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn source_handover_failed_publication_stays_fenced_and_can_retry() {
        let world = tempfile::tempdir().unwrap();
        let transfer = handover();
        let revision = AuthorityRevision::try_from(9).unwrap();
        let journal = Journal::open(world.path()).unwrap();
        // Block the durable writer's temporary sibling before the rename.
        let directory = world.path().join(DIRECTORY);
        fs::create_dir(&directory).unwrap();
        let temporary = directory.join(format!(".{:032x}.{EXTENSION}.durable-new", transfer.owner));
        fs::create_dir(&temporary).unwrap();
        assert!(journal.prepare_versioned(&transfer, revision).is_err());
        assert_eq!(journal.fenced_handover(transfer.owner), Some(transfer.id));
        let other = Handover {
            owner: transfer.owner + 1,
            ..transfer.clone()
        };
        assert!(journal.prepare(&other).is_err());
        assert_eq!(journal.fenced_handover(other.owner), None);
        assert_eq!(
            Journal::open(world.path())
                .unwrap()
                .fenced_handover(transfer.owner),
            None
        );
        fs::remove_dir(temporary).unwrap();
        assert!(journal.recover(transfer.owner, transfer.id + 1).is_err());
        assert_eq!(
            journal.recover(transfer.owner, transfer.id).unwrap(),
            (Phase::Pending, transfer.clone())
        );
        let reopened = Journal::open(world.path()).unwrap();
        assert_eq!(
            reopened.recover(transfer.owner, transfer.id).unwrap(),
            (Phase::Pending, transfer.clone())
        );
        assert!(reopened.prepare(&transfer).is_err());
        assert!(
            reopened
                .prepare_versioned(&transfer, AuthorityRevision::try_from(10).unwrap())
                .is_err()
        );
        reopened
            .retire(
                transfer.owner,
                transfer.id,
                &transfer.destination,
                transfer.record_digest(),
            )
            .unwrap();
        assert_eq!(
            reopened
                .retired_anchor(transfer.owner)
                .unwrap()
                .unwrap()
                .revision,
            revision
        );
        assert_eq!(journal.prepare(&other).unwrap(), Phase::Pending);
    }
}
