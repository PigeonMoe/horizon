//! The compound mods keep on a server player (NeoForge's `Entity.getPersistentData`), read and
//! replaced whole, for mods that carry a player between servers.
use super::{Game, native_server::PlayerRead};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use horizon_core::nbt::{Compound, Limits, Tag};
use mod_api::{
    Identifier, PlayerDataRefusal, PlayerFlagReadError, PlayerRef, TransferProfileReadError,
};
use std::{
    io::{ErrorKind, Read},
    path::Path,
};

/// The key of the mods' compound in a player's saved data (NeoForge's `Entity.PERSISTED_NBT_TAG`).
const PERSISTENT_DATA: &str = "NeoForgeData";
/// The sub-compound a death respawn and the first End exit carry over.
const PLAYER_PERSISTED: &str = "PlayerPersisted";

/// The mods' compound in a player's saved data; empty when there is none.
fn persistent_data(saved: Option<&Tag>) -> Compound {
    match saved.and_then(|t| t.get(PERSISTENT_DATA)) {
        Some(Tag::Compound(data)) => data.clone(),
        _ => Compound::new(),
    }
}

/// Puts `data` in a player's saved data as the mods' compound; an empty one takes the key away.
fn set_persistent_data(saved: &mut Option<Tag>, data: Compound) {
    if !matches!(saved, Some(Tag::Compound(_))) {
        *saved = Some(Tag::Compound(Compound::new()));
    }
    let Some(Tag::Compound(root)) = saved else {
        return;
    };
    if data.is_empty() {
        root.remove(&PERSISTENT_DATA.into());
    } else {
        root.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
    }
}

impl Game<'_> {
    /// The mods' compound on a player: the acting player's saved data is the level's for their
    /// turn, another's is their record's.
    pub(super) fn player_persistent_data(&self, player: PlayerRef) -> Option<Compound> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => persistent_data(self.level.player_extra.as_ref()),
            PlayerRead::Guest(g) => persistent_data(g.saved.as_ref()),
        })
    }

    pub(super) fn player_saved_true_flags(
        &self,
        uuid: u128,
        keys: &[Identifier],
    ) -> Result<Option<Vec<Identifier>>, PlayerFlagReadError> {
        use PlayerFlagReadError as Error;
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return Err(Error::Unavailable);
        }
        let owner = mod_api::content::crash::current_native_mod().ok_or(Error::NotOwned)?;
        if !(1..=512).contains(&keys.len()) {
            return Err(Error::KeyCount);
        }
        if keys.iter().any(|key| key.namespace() != owner) {
            return Err(Error::NotOwned);
        }

        if let Some(player) = self.player(uuid) {
            let saved = match self.player_handle_read(player).ok_or(Error::Unavailable)? {
                PlayerRead::Acting { .. } => self.level.player_extra.as_ref(),
                PlayerRead::Guest(guest) => guest.saved.as_ref(),
            };
            return true_flags(saved, keys).map(Some);
        }
        // A loading, displaced or transferring player must not be read from an
        // older disk checkpoint in place of their live body.
        if self
            .lan
            .as_ref()
            .is_some_and(|host| host.hub.guests.iter().any(|g| g.uuid == uuid))
        {
            return Err(Error::Unavailable);
        }
        read_archived_player(&self.world_root, uuid, true)?
            .map(|saved| true_flags(Some(&saved), keys))
            .transpose()
    }

    pub(super) fn player_transfer_profile_generation(
        &self,
        owner: u128,
        id: &Identifier,
    ) -> Result<Option<u128>, TransferProfileReadError> {
        self.with_transfer_profile_saved(owner, id, |saved| {
            horizon_client::lan::playerdata::transfer_profile_generation(
                &self.world_root,
                owner,
                saved,
                id,
            )
        })
    }

    pub(super) fn player_transfer_profile_payload(
        &self,
        owner: u128,
        id: &Identifier,
        generation: u128,
    ) -> Result<Vec<u8>, TransferProfileReadError> {
        self.with_transfer_profile_saved(owner, id, |saved| {
            horizon_client::lan::playerdata::transfer_profile_payload(
                &self.world_root,
                owner,
                saved,
                id,
                generation,
            )
        })
    }

    fn with_transfer_profile_saved<T>(
        &self,
        owner: u128,
        id: &Identifier,
        read: impl FnOnce(Option<&Tag>) -> Result<T, TransferProfileReadError>,
    ) -> Result<T, TransferProfileReadError> {
        use TransferProfileReadError as Error;
        if !self.is_logical_server() || self.native_lifecycle.epoch().is_none() {
            return Err(Error::Unavailable);
        }
        if mod_api::content::crash::current_native_mod() != Some(id.namespace()) {
            return Err(Error::NotOwned);
        }
        let live_guest = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| guest.uuid == owner && !guest.is_fake())
        });
        if let Some(guest) = live_guest {
            if guest.released {
                return Err(Error::Away);
            }
            if guest.held || guest.handover.is_some() {
                return Err(Error::Transferring);
            }
        }
        if let Some(player) = self.player(owner) {
            let saved = match self.player_handle_read(player).ok_or(Error::Unavailable)? {
                PlayerRead::Acting { .. } => self.level.player_extra.as_ref(),
                PlayerRead::Guest(guest) => guest.saved.as_ref(),
            };
            return read(saved);
        }
        if live_guest.is_some() {
            return Err(Error::Unavailable);
        }
        let saved =
            read_archived_player(&self.world_root, owner, false).map_err(|error| match error {
                PlayerFlagReadError::Unavailable => Error::Unavailable,
                _ => Error::Corrupt,
            })?;
        read(saved.as_ref())
    }

    pub(super) fn player_paper_persistent_value(
        &self,
        player: PlayerRef,
        key: &Identifier,
    ) -> Option<Tag> {
        if self.remote.is_some() {
            return None;
        }
        let saved = match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => self.level.player_extra.as_ref(),
            PlayerRead::Guest(guest) => guest.saved.as_ref(),
        }?;
        saved.get("BukkitValues")?.get(&key.to_string()).cloned()
    }

    /// Replaces the mods' compound on a living player, in their turn so it lands in the record
    /// that is saved. Data that would sit 512 levels deep or more in the player's file
    /// (`isTooDeep`, as `/data merge` refuses it) is refused with nothing written.
    pub(super) fn player_set_persistent_data(
        &mut self,
        player: PlayerRef,
        data: Compound,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        // The compound sits one level under the root of the player's file.
        if data
            .values()
            .any(|tag| horizon_client::nbt_path::too_deep(tag, 2))
        {
            return Err(PlayerDataRefusal::TooDeep);
        }
        let set = self.native_as_player_handle(player, |game| {
            game.journal_player(player.uuid());
            set_persistent_data(&mut game.level.player_extra, data);
        });
        if set {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `PlayerList.respawn` after a death and after a first exit from the End: of the saved
    /// compound only `PlayerPersisted` goes to the new player (NeoForge's
    /// `PlayerEvent.Clone` keeps that sub-compound). Called before the respawn's clone event.
    pub(in crate::game) fn keep_only_player_persisted(&mut self, conn: u32) {
        if let Some(host) = self.lan.as_mut() {
            host.hub
                .guests
                .update_connection(conn, |g| keep_player_persisted(&mut g.saved));
        }
    }
}

fn true_flags(
    saved: Option<&Tag>,
    keys: &[Identifier],
) -> Result<Vec<Identifier>, PlayerFlagReadError> {
    use PlayerFlagReadError::Corrupt;
    let Some(saved) = saved else {
        return Ok(Vec::new());
    };
    if !matches!(saved, Tag::Compound(_)) {
        return Err(Corrupt);
    }
    let Some(data) = saved.get(PERSISTENT_DATA) else {
        return Ok(Vec::new());
    };
    if !matches!(data, Tag::Compound(_)) {
        return Err(Corrupt);
    }
    let Some(flags) = data.get(PLAYER_PERSISTED) else {
        return Ok(Vec::new());
    };
    let Tag::Compound(flags) = flags else {
        return Err(Corrupt);
    };
    Ok(keys
        .iter()
        .filter(|key| flags.get(&key.to_string().as_str().into()) == Some(&Tag::Byte(1)))
        .cloned()
        .collect())
}

fn read_archived_player(
    world: &Path,
    uuid: u128,
    recover_corrupt: bool,
) -> Result<Option<Tag>, PlayerFlagReadError> {
    use PlayerFlagReadError as Error;
    let root = Dir::open_ambient_dir(world, cap_std::ambient_authority())
        .map_err(|_| Error::Unavailable)?;
    let descend = |parent: &Dir, name: &str| match parent.open_dir_nofollow(name) {
        Ok(next) => Ok(Some(next)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Corrupt),
    };
    let Some(players) = descend(&root, "players")? else {
        return Ok(None);
    };
    let Some(directory) = descend(&players, "data")? else {
        return Ok(None);
    };
    let name = horizon_protocol::crypto::uuid_string(uuid);
    let read = |extension: &str| {
        let name = format!("{name}.{extension}");
        let meta = match directory.symlink_metadata(&name) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(Error::Corrupt),
        };
        let max = Limits::saved().max_bytes as u64;
        if !meta.is_file() || meta.len() > max {
            return Err(Error::Corrupt);
        }
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No);
        let file = directory
            .open_with(&name, &options)
            .map_err(|_| Error::Corrupt)?;
        if !file.metadata().map_err(|_| Error::Corrupt)?.is_file() {
            return Err(Error::Corrupt);
        }
        let mut bytes = Vec::new();
        file.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Corrupt)?;
        if bytes.len() as u64 > max {
            return Err(Error::Corrupt);
        }
        let tag = horizon_client::save::decode_dat(bytes).map_err(|_| Error::Corrupt)?;
        if !matches!(tag, Tag::Compound(_)) {
            return Err(Error::Corrupt);
        }
        if let Some(found) = tag.get("UUID")
            && !matches!(found, Tag::IntArray(ints) if ints.as_slice() == horizon_client::lan::playerdata::uuid_ints(uuid))
        {
            return Err(Error::Corrupt);
        }
        Ok(Some(tag))
    };
    let current = read("dat");
    match current {
        Ok(Some(_)) => current,
        Err(_) if !recover_corrupt => current,
        _ => match read("dat_old") {
            Ok(None) => current,
            fallback => fallback,
        },
    }
}

/// Drops every key of the mods' compound but `PlayerPersisted`, whatever its type; an empty
/// compound leaves the key away.
fn keep_player_persisted(saved: &mut Option<Tag>) {
    let data = persistent_data(saved.as_ref());
    let mut kept = Compound::new();
    if let Some(persisted) = data.get(&PLAYER_PERSISTED.into()) {
        kept.insert(PLAYER_PERSISTED.into(), persisted.clone());
    }
    if saved.is_some() {
        set_persistent_data(saved, kept);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved(entries: Vec<(&str, Tag)>) -> Option<Tag> {
        let mut data = Compound::new();
        for (key, tag) in entries {
            data.insert(key.into(), tag);
        }
        let mut saved = Compound::new();
        saved.insert("Other".into(), Tag::Int(1));
        saved.insert(PERSISTENT_DATA.into(), Tag::Compound(data));
        Some(Tag::Compound(saved))
    }

    /// Only `PlayerPersisted` is kept, of any type; the player file's other keys stay.
    #[test]
    fn only_player_persisted_is_kept_whatever_its_type() {
        let mut compound = saved(vec![
            ("lost", Tag::Int(1)),
            (PLAYER_PERSISTED, Tag::String("kept".into())),
        ]);
        keep_player_persisted(&mut compound);
        assert_eq!(
            compound,
            saved(vec![(PLAYER_PERSISTED, Tag::String("kept".into()))])
        );
        let mut none = saved(vec![("lost", Tag::Int(1))]);
        keep_player_persisted(&mut none);
        let Some(Tag::Compound(root)) = &none else {
            panic!("the player file stays")
        };
        assert!(
            root.get(&PERSISTENT_DATA.into()).is_none(),
            "nothing kept, no key"
        );
        assert_eq!(root.get(&"Other".into()), Some(&Tag::Int(1)));
    }
}
