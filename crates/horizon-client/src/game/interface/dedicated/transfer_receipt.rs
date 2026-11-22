//! The destination's receipt is published in the same player file as the profile.
//! Historical receipts acknowledge a completed hand-over without placing another guest.

use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use horizon_client::lan::{
    arrivals,
    guest::Guest,
    source_handover::{AUTHORITY_KEY, AuthorityRevision, ReturnAuthorization},
};
use horizon_core::nbt::{Compound, Limits, Tag};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{ErrorKind, Read},
    path::Path,
};

const RECEIPTS: &str = "HorizonArrivalReceipts";
const STAGED: &str = "HorizonStagedArrivalReceipt";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub owner: u128,
    pub handover: u128,
    source: String,
    destination: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    authority_revision: Option<AuthorityRevision>,
    record_sha256: [u8; 32],
    request_sha256: [u8; 32],
    roster_sha256: [u8; 32],
}

impl Binding {
    pub(super) fn from_import(
        settings: &arrivals::Settings,
        owner: u128,
        request: &Value,
    ) -> Result<Self, String> {
        if request.get("place").and_then(Value::as_bool) != Some(false) {
            return Err("strict profile imports require an explicit place: false".into());
        }
        let record = &request["record"];
        if record["version"].as_u64() != Some(2) || settings.required_profiles.is_empty() {
            return Err("strict profile imports require a version 2 owned profile roster".into());
        }
        let named = record["uuid"].as_str().and_then(arrivals::parse_uuid);
        if named != Some(owner) || owner == 0 {
            return Err("the record is another player's".into());
        }
        let handover = record["handover"]
            .as_str()
            .and_then(arrivals::parse_handover)
            .filter(|id| *id != 0)
            .ok_or("the record needs a hand-over id")?;
        let source = record["source"]
            .as_str()
            .ok_or("the strict record needs its source backend")?;
        let destination = record["destination"]
            .as_str()
            .ok_or("the strict record needs its destination backend")?;
        check_route(settings, source, destination)?;
        Ok(Self {
            owner,
            handover,
            source: source.into(),
            destination: destination.into(),
            authority_revision: record_authority_revision(record)?,
            record_sha256: record_digest(record)?,
            request_sha256: record_digest(request)?,
            roster_sha256: roster_digest(settings)?,
        })
    }

    pub(super) fn from_confirm(settings: &arrivals::Settings, args: &str) -> Result<Self, String> {
        let fields = args.split_whitespace().take(8).collect::<Vec<_>>();
        let [
            owner,
            handover,
            source,
            destination,
            record,
            request,
            remaining @ ..,
        ] = fields.as_slice()
        else {
            return Err("strict player-confirm needs unique id, hand-over id, source, destination, record digest and request digest".into());
        };
        let authority_revision = match remaining {
            [] => None,
            [revision] => Some(parse_authority_revision(revision)?),
            _ => return Err("strict player-confirm takes at most one authority revision".into()),
        };
        check_route(settings, source, destination)?;
        Ok(Self {
            owner: arrivals::parse_uuid(owner)
                .filter(|owner| *owner != 0)
                .ok_or("invalid player unique id")?,
            handover: arrivals::parse_handover(handover)
                .filter(|id| *id != 0)
                .ok_or("invalid hand-over id")?,
            source: (*source).into(),
            destination: (*destination).into(),
            authority_revision,
            record_sha256: parse_digest(record).ok_or("invalid record digest")?,
            request_sha256: parse_digest(request).ok_or("invalid request digest")?,
            roster_sha256: roster_digest(settings)?,
        })
    }

    pub(super) fn add_to_reply(&self, reply: &mut Value) {
        reply["handover"] = json!(arrivals::handover_text(self.handover));
        reply["source"] = json!(self.source);
        reply["destination"] = json!(self.destination);
        reply["record_sha256"] = json!(digest_hex(self.record_sha256));
        reply["request_sha256"] = json!(digest_hex(self.request_sha256));
        if let Some(revision) = self.authority_revision {
            reply["authority_revision"] = json!(u64::from(revision));
        }
    }

    pub(super) fn matches_return(&self, authorization: &ReturnAuthorization) -> bool {
        self.owner == authorization.retired.owner
            && self.handover == authorization.handover
            && self.authority_revision == Some(authorization.revision)
            && self.source == authorization.source
            && self.destination == authorization.destination
            && self.record_sha256 == authorization.record_sha256
            && self.request_sha256 == authorization.request_sha256
    }
}

pub(in crate::game::dedicated) fn parse_authority_revision(
    text: &str,
) -> Result<AuthorityRevision, String> {
    let value = text
        .parse::<u64>()
        .map_err(|_| "invalid transfer authority revision")?;
    AuthorityRevision::try_from(value).map_err(str::to_owned)
}

pub(in crate::game::dedicated) fn record_authority_revision(
    record: &Value,
) -> Result<Option<AuthorityRevision>, String> {
    record
        .get("authority_revision")
        .map(|value| {
            let value = value
                .as_u64()
                .ok_or("invalid transfer authority revision")?;
            AuthorityRevision::try_from(value).map_err(str::to_owned)
        })
        .transpose()
}

pub(in crate::game::dedicated) fn record_digest(record: &Value) -> Result<[u8; 32], String> {
    serde_json::to_vec(record)
        .map(|bytes| Sha256::digest(bytes).into())
        .map_err(|error| error.to_string())
}

fn roster_digest(settings: &arrivals::Settings) -> Result<[u8; 32], String> {
    let mut entries = settings
        .required_profiles
        .iter()
        .map(|(id, version)| (id.to_string(), version))
        .collect::<Vec<_>>();
    entries.sort_unstable();
    serde_json::to_vec(&entries)
        .map(|bytes| Sha256::digest(bytes).into())
        .map_err(|error| error.to_string())
}

fn check_route(
    settings: &arrivals::Settings,
    source: &str,
    destination: &str,
) -> Result<(), String> {
    if source == destination
        || settings.backend_id.as_deref() != Some(destination)
        || !settings.allowed_sources.contains(source)
    {
        return Err("the strict hand-over route is not approved for this destination".into());
    }
    Ok(())
}

pub(in crate::game::dedicated) fn digest_hex(digest: [u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn parse_digest(input: &str) -> Option<[u8; 32]> {
    if input.len() != 64
        || !input
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return None;
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&input[2 * index..2 * index + 2], 16).ok()?;
    }
    Some(digest)
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    version: u8,
    pub binding: Binding,
    announced: bool,
    recentered: bool,
}

impl Receipt {
    pub(super) fn new(binding: Binding, announced: bool, recentered: bool) -> Self {
        Self {
            version: if binding.authority_revision.is_some() {
                2
            } else {
                1
            },
            binding,
            announced,
            recentered,
        }
    }

    pub(super) fn commit_reply(&self, currently_placed: bool, retried: bool) -> Value {
        let mut reply = json!({
            "ok": true, "placed": true, "announced": self.announced, "recentered": self.recentered,
            "retried": retried, "already_committed": retried, "currently_placed": currently_placed,
        });
        self.binding.add_to_reply(&mut reply);
        reply
    }
}

/// Temporarily remove destination-local metadata before a portable player record is made.
pub(in crate::game::dedicated) struct Hidden {
    receipts: Option<Tag>,
    staged: Option<Tag>,
    authority: Option<Tag>,
}

impl Hidden {
    pub(in crate::game::dedicated) fn take(guest: &mut Guest) -> Self {
        if let Some(Tag::Compound(root)) = guest.saved.as_mut() {
            Self {
                receipts: root.remove(&RECEIPTS.into()),
                staged: root.remove(&STAGED.into()),
                authority: root.remove(&AUTHORITY_KEY.into()),
            }
        } else {
            Self {
                receipts: None,
                staged: None,
                authority: None,
            }
        }
    }

    pub(in crate::game::dedicated) fn restore(self, guest: &mut Guest) {
        if self.receipts.is_none() && self.staged.is_none() && self.authority.is_none() {
            return;
        }
        let root = root_for(guest);
        for (key, value) in [
            (RECEIPTS, self.receipts),
            (STAGED, self.staged),
            (AUTHORITY_KEY, self.authority),
        ] {
            if let Some(value) = value {
                root.insert(key.into(), value);
            }
        }
    }
}

fn root_for(guest: &mut Guest) -> &mut Compound {
    let tag = guest
        .saved
        .get_or_insert_with(|| Tag::Compound(Compound::new()));
    let Tag::Compound(root) = tag else {
        unreachable!("a guest has compound player data");
    };
    root
}

pub(super) fn set_staged(guest: &mut Guest, receipt: &Receipt) -> Result<(), String> {
    let encoded = encode(receipt)?;
    root_for(guest).insert(STAGED.into(), encoded);
    Ok(())
}

pub(super) fn staged(guest: &Guest) -> Result<Option<Receipt>, String> {
    guest
        .saved
        .as_ref()
        .and_then(|tag| tag.get(STAGED))
        .map(decode)
        .transpose()
}

fn encode(receipt: &Receipt) -> Result<Tag, String> {
    serde_json::to_vec(receipt)
        .map(Tag::ByteArray)
        .map_err(|error| error.to_string())
}

fn decode(value: &Tag) -> Result<Receipt, String> {
    let Tag::ByteArray(bytes) = value else {
        return Err("arrival receipt is not a byte array".into());
    };
    let receipt: Receipt =
        serde_json::from_slice(bytes).map_err(|_| "arrival receipt is corrupt")?;
    if !matches!(
        (receipt.version, receipt.binding.authority_revision),
        (1, None) | (2, Some(_))
    ) {
        return Err("arrival receipt version is not supported".into());
    }
    Ok(receipt)
}

/// Existing receipts are append-only inside the owner file. The same file activates the
/// incoming SFP1 data and records completion. No second commit file can get out of step.
pub(super) fn append_receipt(
    world: &Path,
    guest: &mut Guest,
    receipt: &Receipt,
    returning: Option<&ReturnAuthorization>,
) -> Result<Hidden, String> {
    let mut history = read_history(world, guest.uuid)?;
    let encoded = serde_json::to_vec(receipt).map_err(|error| error.to_string())?;
    let authority = returning
        .map(|authorization| authorization.marker(&encoded))
        .transpose()
        .map_err(|error| error.to_string())?;
    if history
        .entries
        .insert(
            arrivals::handover_text(receipt.binding.handover)
                .as_str()
                .into(),
            Tag::ByteArray(encoded),
        )
        .is_some()
    {
        return Err("the hand-over already has a durable arrival receipt".into());
    }
    let previous = Hidden::take(guest);
    let root = root_for(guest);
    root.insert(RECEIPTS.into(), Tag::Compound(history.entries));
    if let Some(authority) = authority {
        root.insert(AUTHORITY_KEY.into(), authority);
    }
    Ok(previous)
}

pub(super) fn lookup(world: &Path, binding: &Binding) -> Result<Option<Receipt>, String> {
    let history = read_history(world, binding.owner)?;
    let key = arrivals::handover_text(binding.handover);
    let Some(receipt) = history
        .entries
        .get(&key.as_str().into())
        .map(decode)
        .transpose()?
    else {
        return Ok(None);
    };
    if receipt.binding != *binding {
        return Err("the strict hand-over differs from its durable arrival receipt".into());
    }
    // A writer can report a directory sync error after publishing the new player file.
    // A matching retry acknowledges that rename only after its directory is synced.
    #[cfg(unix)]
    if let Some(directory) = history.directory {
        directory
            .open(".")
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    Ok(Some(receipt))
}

#[derive(Default)]
struct History {
    entries: Compound,
    #[cfg(unix)]
    directory: Option<Dir>,
}

fn read_history(world: &Path, owner: u128) -> Result<History, String> {
    let root = Dir::open_ambient_dir(world, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let child = |parent: &Dir, name| match parent.open_dir_nofollow(name) {
        Ok(dir) => Ok(Some(dir)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    };
    let Some(players) = child(&root, "players")? else {
        return Ok(History::default());
    };
    let Some(directory) = child(&players, "data")? else {
        return Ok(History::default());
    };
    let owner_name = horizon_protocol::crypto::uuid_string(owner);
    let read = |ext| read_player_file(&directory, &format!("{owner_name}.{ext}"));
    // A missing current file can be the safeReplaceFile rename window. A corrupt
    // current file could conceal a newer receipt and therefore never falls back.
    let file = match read("dat")? {
        Some(file) => Some(file),
        None => read("dat_old")?,
    };
    let entries = match file.as_ref().and_then(|file| file.get(RECEIPTS)) {
        Some(Tag::Compound(entries)) => entries.clone(),
        None => Compound::new(),
        Some(_) => return Err("arrival receipt history is corrupt".into()),
    };
    if let Some(found) = file.as_ref().and_then(|file| file.get("UUID"))
        && !matches!(found, Tag::IntArray(ints) if ints.as_slice() == horizon_client::lan::playerdata::uuid_ints(owner))
    {
        return Err("arrival receipt history belongs to another player".into());
    }
    for (key, value) in &entries {
        let binding = decode(value)?.binding;
        if binding.owner != owner
            || *key != arrivals::handover_text(binding.handover).as_str().into()
        {
            return Err("arrival receipt does not match its owner and hand-over".into());
        }
    }
    Ok(History {
        entries,
        #[cfg(unix)]
        directory: Some(directory),
    })
}

fn read_player_file(directory: &Dir, name: &str) -> Result<Option<Tag>, String> {
    let max = Limits::saved().max_bytes as u64;
    let metadata = match directory.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    if !metadata.is_file() || metadata.len() > max {
        return Err("arrival receipt player file is invalid".into());
    }
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = match directory.open_with(name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > max {
        return Err("arrival receipt player file is invalid".into());
    }
    let mut bytes = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > max {
        return Err("arrival receipt player file is too large".into());
    }
    match horizon_client::save::decode_dat(bytes) {
        Ok(tag @ Tag::Compound(_)) => Ok(Some(tag)),
        _ => Err("arrival receipt player file is corrupt".into()),
    }
}
