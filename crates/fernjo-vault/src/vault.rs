//! The `Vault`: create, open, unlock, lock, and the encrypted atomic save.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{self, DataKey, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::entry::{Entry, EntryId, EntrySummary, EntryUpdate, NewEntry, Secret, non_empty};
use crate::error::VaultError;
use crate::format::{
    HEADER_LEN, Header, KdfParams, MAX_FILE_LEN, OFF_WRAPPED_KEY, WRAPPED_KEY_LEN, read_capped,
};
use crate::generation::GenerationStore;

/// Identifier of a vault, stored in its header; keys the OS keychain item (Task 3).
pub type VaultId = Uuid;

/// Minimum master password length in characters, counted after NFC normalisation.
const MIN_PASSWORD_CHARS: usize = 8;

/// The decrypted body: `{ "generation": N, "entries": [...] }`. A body written before the
/// counter existed has no `generation` and reads as 0; unknown fields are ignored, so an older
/// build can still read a file written by this one (the header version stays 1).
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Body {
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub entries: Vec<Entry>,
}

/// What `save` serialises: the same JSON shape as [`Body`], borrowing the entries.
#[derive(Serialize)]
struct BodyOut<'a> {
    generation: u64,
    entries: &'a [Entry],
}

/// What proves to `unlock_inner` that the caller may open the vault.
enum KeySource<'a> {
    Password(&'a SecretString),
    Key(&'a DataKey),
}

struct Unlocked {
    key: DataKey,
    body: Body,
}

/// An encrypted vault file. Locked vaults hold no key and no decrypted entries.
pub struct Vault {
    path: PathBuf,
    header: Box<Header>,
    /// The header bytes this handle last read from or wrote to the file. `save` refuses to
    /// overwrite a file that no longer starts with them (see `check_unchanged_on_disk`).
    /// It differs from `header.encode()` only while `change_password` saves.
    disk_header: Box<[u8; HEADER_LEN]>,
    unlocked: Option<Unlocked>,
    /// The highest generation this handle has seen or written, also in the device marks. A file
    /// below it is a rollback. Never lowered (accepting an older file keeps it).
    floor: u64,
    generations: Option<Arc<dyn GenerationStore>>,
    /// The cost `change_password` raises a weaker stored cost to (never lowers a stronger one).
    upgrade_target: KdfParams,
}

impl fmt::Debug for Vault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vault")
            .field("id", &self.header.id)
            .field("path", &self.path)
            .field("unlocked", &self.unlocked.is_some())
            .finish_non_exhaustive()
    }
}

/// NFC-normalises the password and does nothing else (no trimming, byte-exact).
pub(crate) fn normalise(password: &SecretString) -> Zeroizing<String> {
    Zeroizing::new(password.expose_secret().nfc().collect::<String>())
}

/// AAD of the key wrap: everything before the wrapped key. The wrapped key is covered by
/// its own tag and, together with every other header byte, by the body's AAD.
fn key_wrap_aad(header_bytes: &[u8]) -> &[u8] {
    &header_bytes[..OFF_WRAPPED_KEY]
}

impl Vault {
    /// Creates a new vault file with the default (about 0.5 s) KDF parameters and leaves
    /// it unlocked.
    ///
    /// # Errors
    /// `weak_password`, `already_exists`, `save_failed`.
    pub fn create(path: &Path, password: &SecretString) -> Result<Self, VaultError> {
        Self::create_with(path, password, KdfParams::default())
    }

    /// Like [`Vault::create`] with explicit KDF parameters (tests use `KdfParams::TEST`).
    ///
    /// # Errors
    /// `weak_password`, `invalid_kdf_params`, `already_exists`, `save_failed`.
    pub fn create_with(
        path: &Path,
        password: &SecretString,
        kdf: KdfParams,
    ) -> Result<Self, VaultError> {
        let password = normalise(password);
        if password.chars().count() < MIN_PASSWORD_CHARS {
            return Err(VaultError::WeakPassword);
        }
        kdf.validate()?;
        if path.exists() {
            return Err(VaultError::AlreadyExists);
        }
        let salt: [u8; SALT_LEN] = crypto::random_array()?;
        let wrap_nonce: [u8; NONCE_LEN] = crypto::random_array()?;
        let key = DataKey::random()?;
        let mut header = Header {
            id: Uuid::new_v4(),
            kdf,
            salt,
            wrap_nonce,
            wrapped_key: [0; WRAPPED_KEY_LEN],
            body_nonce: [0; NONCE_LEN],
        };
        let kek = crypto::derive_key(password.as_bytes(), kdf, &salt)?;
        let aad = header.encode();
        let wrapped = crypto::seal(&kek, &wrap_nonce, key_wrap_aad(&aad), key.as_bytes())?;
        header.wrapped_key = wrapped
            .try_into()
            .map_err(|_| VaultError::SaveFailed("unexpected wrapped key size".to_owned()))?;
        let mut vault = Self {
            path: path.to_path_buf(),
            disk_header: Box::new(header.encode()),
            header: Box::new(header),
            unlocked: Some(Unlocked {
                key,
                body: Body::default(),
            }),
            floor: 0,
            generations: None,
            upgrade_target: kdf,
        };
        vault.save()?;
        Ok(vault)
    }

    /// Opens an existing vault file, locked. `Ok(None)` if the file does not exist.
    ///
    /// # Errors
    /// `unreadable`, `newer_format`, `invalid_kdf_params`.
    pub fn open(path: &Path) -> Result<Option<Self>, VaultError> {
        fernjo_fs::sweep_stale_temps(path, STALE_TEMP_AGE);
        let bytes = match read_capped(path, MAX_FILE_LEN) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(VaultError::Unreadable(e.to_string())),
        };
        let (header, _) = Header::parse(&bytes)?;
        Ok(Some(Self {
            path: path.to_path_buf(),
            disk_header: Box::new(header.encode()),
            header: Box::new(header),
            unlocked: None,
            floor: 0,
            generations: None,
            upgrade_target: KdfParams::DEFAULT,
        }))
    }

    #[must_use]
    pub fn id(&self) -> VaultId {
        self.header.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn is_unlocked(&self) -> bool {
        self.unlocked.is_some()
    }

    /// Attaches the device marks (see [`GenerationStore`]): the floor becomes the higher of its
    /// own value and the mark, and an unlocked vault raises the mark to its generation. Without
    /// a store the handle still remembers what it saw while it lives.
    pub fn attach_generation_store(&mut self, store: Arc<dyn GenerationStore>) {
        if let Some(mark) = store.get(self.header.id) {
            self.floor = self.floor.max(mark);
        }
        if let Some(u) = &self.unlocked {
            store.raise(self.header.id, u.body.generation);
        }
        self.generations = Some(store);
    }

    /// The generation of the open body (it counts the saves of the vault), `None` while locked.
    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        self.unlocked.as_ref().map(|u| u.body.generation)
    }

    /// Unlocks with the master password. The file is read again, so a vault updated on
    /// disk by a sync tool is picked up; a file with another vault id is refused. A file whose
    /// generation is below the highest this device has seen is refused as `rolled_back`, and
    /// only after the password proved genuine. On any error the vault stays locked.
    ///
    /// # Errors
    /// `wrong_password`, `tampered`, `unreadable`, `newer_format`, `invalid_kdf_params`,
    /// `rolled_back`.
    pub fn unlock(&mut self, password: &SecretString) -> Result<(), VaultError> {
        self.unlock_inner(&KeySource::Password(password), false)
    }

    /// Like [`Vault::unlock`], but opens a rolled-back file too (the user decided to take the
    /// older version). The floor is not lowered, and the file is re-stamped at once with a
    /// best-effort save (generation `floor + 1`, `.bak` keeps the file as it was); a failed
    /// re-stamp keeps the unlock and the question returns on the next start.
    ///
    /// # Errors
    /// As [`Vault::unlock`] except `rolled_back`.
    pub fn unlock_accepting_older(&mut self, password: &SecretString) -> Result<(), VaultError> {
        self.unlock_inner(&KeySource::Password(password), true)
    }

    /// Unlocks with the data key (for example read from the OS keychain). Same file
    /// handling as [`Vault::unlock`]; a key that does not open the body is refused, and so is
    /// a rolled-back file (the automatic unlock never accepts an older version).
    ///
    /// # Errors
    /// `tampered` (the key does not fit the file), `unreadable`, `newer_format`,
    /// `invalid_kdf_params`, `rolled_back`.
    pub fn unlock_with_key(&mut self, key: &DataKey) -> Result<(), VaultError> {
        self.unlock_inner(&KeySource::Key(key), false)
    }

    fn unlock_inner(
        &mut self,
        source: &KeySource<'_>,
        accept_older: bool,
    ) -> Result<(), VaultError> {
        let bytes = read_capped(&self.path, MAX_FILE_LEN)
            .map_err(|e| VaultError::Unreadable(e.to_string()))?;
        let (header, ciphertext) = Header::parse(&bytes)?;
        if header.id != self.header.id {
            return Err(VaultError::Unreadable(
                "the vault file was replaced by a different vault".to_owned(),
            ));
        }
        let header_bytes = header.encode();
        let key = match source {
            KeySource::Password(password) => {
                let password = normalise(password);
                let kek = crypto::derive_key(password.as_bytes(), header.kdf, &header.salt)?;
                let raw = crypto::open(
                    &kek,
                    &header.wrap_nonce,
                    key_wrap_aad(&header_bytes),
                    &header.wrapped_key,
                )
                .map_err(|()| VaultError::WrongPassword)?;
                let key_bytes: [u8; 32] = raw
                    .as_slice()
                    .try_into()
                    .map_err(|_| VaultError::Tampered)?;
                DataKey::from_bytes(key_bytes)
            }
            KeySource::Key(key) => DataKey::from_bytes(*key.as_bytes()),
        };
        let body = open_body(&key, &header, &header_bytes, ciphertext)?;

        // Only now, with the password or key proven genuine, is a rollback revealed.
        let mark = self
            .generations
            .as_ref()
            .and_then(|s| s.get(header.id))
            .unwrap_or(0);
        self.floor = self.floor.max(mark);
        let rolled_back = body.generation < self.floor;
        if rolled_back && !accept_older {
            return Err(VaultError::RolledBack {
                found: body.generation,
                seen: self.floor,
            });
        }
        self.floor = self.floor.max(body.generation);
        if let Some(store) = &self.generations {
            store.raise(header.id, self.floor);
        }
        *self.disk_header = header.encode();
        *self.header = header;
        self.unlocked = Some(Unlocked { key, body });
        if rolled_back {
            // Re-stamp above the floor so this device and the others stop asking. Best
            // effort: on failure the vault stays open and the next start asks again.
            let _ = self.save();
        }
        Ok(())
    }

    /// Forgets the data key and the decrypted entries (the key is zeroised on drop).
    pub fn lock(&mut self) {
        self.unlocked = None;
    }

    /// A copy of the data key of an unlocked vault (for the OS keychain store).
    ///
    /// # Errors
    /// `vault_locked`.
    pub fn data_key(&self) -> Result<DataKey, VaultError> {
        self.unlocked
            .as_ref()
            .map(|u| DataKey::from_bytes(*u.key.as_bytes()))
            .ok_or(VaultError::VaultLocked)
    }

    fn unlocked(&self) -> Result<&Unlocked, VaultError> {
        self.unlocked.as_ref().ok_or(VaultError::VaultLocked)
    }

    /// Lists all entries without their secrets.
    ///
    /// # Errors
    /// `vault_locked`.
    pub fn list(&self) -> Result<Vec<EntrySummary>, VaultError> {
        Ok(self
            .unlocked()?
            .body
            .entries
            .iter()
            .map(Entry::summary)
            .collect())
    }

    fn entry(&self, id: EntryId) -> Result<&Entry, VaultError> {
        self.unlocked()?
            .body
            .entries
            .iter()
            .find(|e| e.id == id)
            .ok_or(VaultError::NotFound)
    }

    /// One entry without its secret.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`.
    pub fn get(&self, id: EntryId) -> Result<EntrySummary, VaultError> {
        self.entry(id).map(Entry::summary)
    }

    /// A copy of the entry's secret. The only way a secret leaves the vault.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`.
    pub fn reveal(&self, id: EntryId) -> Result<Secret, VaultError> {
        self.entry(id).map(|e| e.secret.clone())
    }

    /// Adds an entry and saves.
    ///
    /// # Errors
    /// `vault_locked`, `empty_name`, `changed_on_disk`, `save_failed` (memory unchanged).
    pub fn add(&mut self, new: NewEntry) -> Result<EntryId, VaultError> {
        let name = checked_name(&new.name)?;
        let id = Uuid::new_v4();
        self.mutate(|entries| {
            entries.push(entry_from(id, name, new));
            Ok(())
        })?;
        Ok(id)
    }

    /// Adds an entry under a given id (restore from a backup) and saves.
    ///
    /// # Errors
    /// `vault_locked`, `empty_name`, `duplicate_entry`, `changed_on_disk`, `save_failed`
    /// (memory unchanged).
    pub fn add_with_id(&mut self, id: EntryId, new: NewEntry) -> Result<(), VaultError> {
        let name = checked_name(&new.name)?;
        self.mutate(|entries| {
            if entries.iter().any(|e| e.id == id) {
                return Err(VaultError::DuplicateEntry);
            }
            entries.push(entry_from(id, name, new));
            Ok(())
        })
    }

    /// Adds many entries under given ids with ONE save (an import): the ids and names are
    /// validated first, so a bad batch changes nothing, and `.bak` afterwards holds the vault
    /// as it was before the call.
    ///
    /// # Errors
    /// `vault_locked`, `empty_name`, `duplicate_entry` (an id repeated in `entries` or already
    /// in the vault), `changed_on_disk`, `save_failed` (memory unchanged).
    pub fn add_entries_with_ids(
        &mut self,
        entries: Vec<(EntryId, NewEntry)>,
    ) -> Result<(), VaultError> {
        self.unlocked()?;
        let mut list = Vec::with_capacity(entries.len());
        for (id, new) in entries {
            let name = checked_name(&new.name)?;
            list.push(entry_from(id, name, new));
        }
        self.mutate(|current| {
            let mut seen: std::collections::HashSet<EntryId> =
                current.iter().map(|e| e.id).collect();
            if list.iter().any(|e| !seen.insert(e.id)) {
                return Err(VaultError::DuplicateEntry);
            }
            current.extend(list);
            Ok(())
        })
    }

    /// Deletes many entries with ONE save (rolling back an import). Every id must exist;
    /// otherwise nothing changes.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`, `changed_on_disk`, `save_failed` (memory unchanged).
    pub fn remove_entries(&mut self, ids: &[EntryId]) -> Result<(), VaultError> {
        self.mutate(|entries| {
            let doomed: std::collections::HashSet<EntryId> = ids.iter().copied().collect();
            let before = entries.len();
            entries.retain(|e| !doomed.contains(&e.id));
            if before - entries.len() != doomed.len() {
                return Err(VaultError::NotFound);
            }
            Ok(())
        })
    }

    /// Every entry with its secret, for an encrypted backup. Secret material leaves the
    /// vault here: the caller must seal it immediately.
    ///
    /// # Errors
    /// `vault_locked`.
    pub fn export_entries(&self) -> Result<Vec<(EntryId, NewEntry)>, VaultError> {
        Ok(self
            .unlocked()?
            .body
            .entries
            .iter()
            .map(export_pair)
            .collect())
    }

    /// Replaces all entries by `entries` with one save (restore from a backup).
    ///
    /// # Errors
    /// `vault_locked`, `empty_name`, `duplicate_entry`, `changed_on_disk`, `save_failed`
    /// (memory unchanged).
    pub fn replace_entries(&mut self, entries: Vec<(EntryId, NewEntry)>) -> Result<(), VaultError> {
        self.unlocked()?;
        let mut seen = std::collections::HashSet::new();
        let mut list = Vec::with_capacity(entries.len());
        for (id, new) in entries {
            let name = checked_name(&new.name)?;
            if !seen.insert(id) {
                return Err(VaultError::DuplicateEntry);
            }
            list.push(entry_from(id, name, new));
        }
        self.mutate(|current| {
            *current = list;
            Ok(())
        })
    }

    /// Applies a partial update and saves.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`, `empty_name`, `changed_on_disk`, `save_failed` (memory unchanged).
    pub fn update(&mut self, id: EntryId, update: EntryUpdate) -> Result<(), VaultError> {
        let name = update.name.as_deref().map(checked_name).transpose()?;
        self.mutate(|entries| {
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or(VaultError::NotFound)?;
            if let Some(name) = name {
                entry.name = name;
            }
            if let Some(username) = update.username {
                entry.username = non_empty(Some(username));
            }
            if let Some(domain) = update.domain {
                entry.domain = non_empty(Some(domain));
            }
            if let Some(secret) = update.secret {
                entry.secret = secret;
            }
            if let Some(notes) = update.notes {
                entry.notes = notes;
            }
            Ok(())
        })
    }

    /// Deletes an entry and saves.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`, `changed_on_disk`, `save_failed` (memory unchanged).
    pub fn delete(&mut self, id: EntryId) -> Result<(), VaultError> {
        self.mutate(|entries| {
            let before = entries.len();
            entries.retain(|e| e.id != id);
            if entries.len() == before {
                return Err(VaultError::NotFound);
            }
            Ok(())
        })
    }

    /// The vault file as this handle last read or wrote it, read from disk with the size cap
    /// (for sync: the local side of a merge and what is pushed). A file that no longer starts
    /// with the header this handle knows (an older copy, another vault's file or anything else
    /// a writer of the data folder put there) is `changed_on_disk`, the same rule the save guard
    /// applies: such a file is never handed out to be pushed. Lock and unlock to load it.
    ///
    /// # Errors
    /// `unreadable` (missing, unreadable or larger than `MAX_FILE_LEN`), `changed_on_disk`.
    pub fn file_bytes(&self) -> Result<Vec<u8>, VaultError> {
        let bytes = read_capped(&self.path, MAX_FILE_LEN)
            .map_err(|e| VaultError::Unreadable(e.to_string()))?;
        if bytes.get(..HEADER_LEN) != Some(self.disk_header.as_slice()) {
            return Err(VaultError::ChangedOnDisk);
        }
        Ok(bytes)
    }

    /// Parses `bytes` as a file of this vault: size cap, header (KDF bounds, newer-format rule)
    /// and vault id. Nothing is decrypted.
    fn parse_foreign<'a>(&self, bytes: &'a [u8]) -> Result<(Header, &'a [u8]), VaultError> {
        check_foreign_len(bytes)?;
        let (header, ciphertext) = Header::parse(bytes)?;
        if header.id != self.header.id {
            return Err(VaultError::Unreadable(
                "the file belongs to a different vault".to_owned(),
            ));
        }
        Ok((header, ciphertext))
    }

    /// Like `parse_foreign`, and opens the body with the held data key. A body that does not
    /// open is `unreadable` here: the data key is stable across password changes, so it is a
    /// different vault or a damaged file, never a reason to write.
    fn open_foreign(&self, bytes: &[u8]) -> Result<(Header, Body), VaultError> {
        let key = &self.unlocked()?.key;
        let (header, ciphertext) = self.parse_foreign(bytes)?;
        let body = open_body(key, &header, &header.encode(), ciphertext).map_err(|_| {
            VaultError::Unreadable("the file does not open with this vault's data key".to_owned())
        })?;
        Ok((header, body))
    }

    /// Makes `bytes` the vault file as they are (sync rule: only the other side changed the
    /// vault), also while the vault is locked. Nothing is written when `bytes` are larger than
    /// `MAX_FILE_LEN` or belong to another vault (`unreadable`), are a newer format, do not
    /// open with the held key (unlocked: `unreadable`), have a generation below the floor
    /// (unlocked: `rolled_back`), or when the file on disk is not the one this handle knows
    /// (`changed_on_disk`). The file is written atomically with a `.bak`. An unlocked vault
    /// stays unlocked with the new entries and keeps its key; a locked vault stays locked and
    /// neither the floor nor the device mark is touched (the next unlock does that). The floor
    /// and the mark are never lowered.
    ///
    /// # Errors
    /// `unreadable`, `newer_format`, `invalid_kdf_params`, `rolled_back`, `changed_on_disk`,
    /// `save_failed`.
    pub fn adopt_file(&mut self, bytes: &[u8]) -> Result<(), VaultError> {
        let (header, body) = if self.unlocked.is_some() {
            let (header, body) = self.open_foreign(bytes)?;
            if body.generation < self.floor {
                return Err(VaultError::RolledBack {
                    found: body.generation,
                    seen: self.floor,
                });
            }
            (header, Some(body))
        } else {
            (self.parse_foreign(bytes)?.0, None)
        };
        self.check_unchanged_on_disk()?;
        write_atomic(&self.path, bytes).map_err(|e| VaultError::SaveFailed(e.to_string()))?;
        *self.disk_header = header.encode();
        *self.header = header;
        if let Some(body) = body {
            self.floor = self.floor.max(body.generation);
            if let Some(store) = &self.generations {
                store.raise(self.header.id, self.floor);
            }
            if let Some(u) = self.unlocked.as_mut() {
                u.body = body;
            }
        }
        Ok(())
    }

    /// Makes `bytes` the vault file even when they belong to another vault than this handle's
    /// (sync join: the joining device takes the group's vault, replacing a vault it created on
    /// its own). The same guards as [`Vault::adopt_file`] apply to the bytes (size cap, header
    /// with the KDF bounds, newer-format rule) and to the file on disk (`changed_on_disk`), and
    /// the write is atomic with a `.bak`. The handle always ends locked, because a key held for
    /// the old file does not open the new one: it takes the header (so the vault id may change),
    /// and its floor becomes the device mark of the new vault id (never lowered, never written
    /// here; a different id starts from that mark alone). Nothing is written on an error.
    ///
    /// # Errors
    /// `unreadable` (also a KDF cost above the cap), `newer_format`, `changed_on_disk`,
    /// `save_failed`.
    pub fn replace_with_file(&mut self, bytes: &[u8]) -> Result<(), VaultError> {
        check_foreign_len(bytes)?;
        let (header, _) = Header::parse(bytes)?;
        self.check_unchanged_on_disk()?;
        write_atomic(&self.path, bytes).map_err(|e| VaultError::SaveFailed(e.to_string()))?;
        let mark = self
            .generations
            .as_ref()
            .and_then(|s| s.get(header.id))
            .unwrap_or(0);
        self.floor = if header.id == self.header.id {
            self.floor.max(mark)
        } else {
            mark
        };
        *self.disk_header = header.encode();
        *self.header = header;
        self.unlocked = None;
        Ok(())
    }

    /// Creates the vault file at `path` from `bytes` (sync join on a device that has no vault
    /// yet) and returns the handle, locked. The bytes are checked like in
    /// [`Vault::replace_with_file`]; an existing file (also an unreadable one) is never
    /// overwritten (`already_exists`). Nothing is written on an error.
    ///
    /// # Errors
    /// `unreadable`, `newer_format`, `invalid_kdf_params`, `already_exists`, `save_failed`.
    pub fn install_file(path: &Path, bytes: &[u8]) -> Result<Self, VaultError> {
        check_foreign_len(bytes)?;
        let (header, _) = Header::parse(bytes)?;
        // Unreadable metadata counts as "exists": never risk overwriting.
        if path.try_exists().unwrap_or(true) {
            return Err(VaultError::AlreadyExists);
        }
        write_atomic(path, bytes).map_err(|e| VaultError::SaveFailed(e.to_string()))?;
        Ok(Self {
            path: path.to_path_buf(),
            disk_header: Box::new(header.encode()),
            header: Box::new(header),
            unlocked: None,
            floor: 0,
            generations: None,
            upgrade_target: KdfParams::DEFAULT,
        })
    }

    /// Writes the merge of both sides (sync rule: both changed the vault; needs an unlocked
    /// vault): `entries` under the held data key, with the header of `head_bytes` (its
    /// password wrap and KDF cost) and a generation above both this handle's and the head's.
    /// `head_bytes` must belong to this vault and open with the held key, otherwise nothing is
    /// written (`unreadable`). `base_generation` is the generation of the base file both sides
    /// started from (0 without one): a head below it is an older file of this vault played back
    /// (every save since the base raised the generation), so nothing is written (`rolled_back`).
    /// The head is not compared with this handle's floor or the device mark: those include this
    /// device's own saves since the base, so a head that saved fewer times is still newer than
    /// the base. Entries are validated like [`Vault::replace_entries`]. On any error the handle
    /// and the file stay as they were.
    ///
    /// # Errors
    /// `vault_locked`, `unreadable`, `newer_format`, `invalid_kdf_params`, `rolled_back`,
    /// `empty_name`, `duplicate_entry`, `changed_on_disk`, `save_failed`.
    pub fn adopt_merged(
        &mut self,
        head_bytes: &[u8],
        base_generation: u64,
        entries: Vec<(EntryId, NewEntry)>,
    ) -> Result<(), VaultError> {
        let (head_header, head_body) = self.open_foreign(head_bytes)?;
        if head_body.generation < base_generation {
            return Err(VaultError::RolledBack {
                found: head_body.generation,
                seen: base_generation,
            });
        }
        let mut seen = std::collections::HashSet::new();
        let mut list = Vec::with_capacity(entries.len());
        for (id, new) in entries {
            let name = checked_name(&new.name)?;
            if !seen.insert(id) {
                return Err(VaultError::DuplicateEntry);
            }
            list.push(entry_from(id, name, new));
        }
        let previous_header = std::mem::replace(&mut self.header, Box::new(head_header));
        let previous_floor = self.floor;
        self.floor = self.floor.max(head_body.generation);
        let previous_entries = match self.unlocked.as_mut() {
            Some(u) => std::mem::replace(&mut u.body.entries, list),
            None => Vec::new(),
        };
        let result = self.save();
        if result.is_err() {
            self.header = previous_header;
            self.floor = previous_floor;
            if let Some(u) = self.unlocked.as_mut() {
                u.body.entries = previous_entries;
            }
        }
        result
    }

    /// Runs `f` on the entry list, saves, and restores the previous list if `f` or the
    /// save fails, so memory always matches what is on disk.
    fn mutate<R>(
        &mut self,
        f: impl FnOnce(&mut Vec<Entry>) -> Result<R, VaultError>,
    ) -> Result<R, VaultError> {
        let snapshot = self.unlocked()?.body.entries.clone();
        let result = match self.unlocked.as_mut() {
            Some(u) => f(&mut u.body.entries),
            None => Err(VaultError::VaultLocked),
        };
        let result = result.and_then(|r| self.save().map(|()| r));
        if result.is_err()
            && let Some(u) = self.unlocked.as_mut()
        {
            u.body.entries = snapshot;
        }
        result
    }

    /// Changes the master password. Verifies the old one, then uses a new salt and wrap
    /// nonce, re-wraps the unchanged data key and saves once (the save re-seals the body
    /// under the same data key with a fresh nonce, because the header is its AAD). A stored KDF
    /// cost below the current default is raised to it (the new password is derived at the
    /// new cost; a stronger stored cost stays). A
    /// failure leaves the file and the header in memory as they were.
    ///
    /// # Errors
    /// `vault_locked`, `wrong_password`, `weak_password`, `changed_on_disk`, `save_failed`.
    pub fn change_password(
        &mut self,
        old: &SecretString,
        new: &SecretString,
    ) -> Result<(), VaultError> {
        let key = DataKey::from_bytes(*self.unlocked()?.key.as_bytes());
        let old = normalise(old);
        let kek = crypto::derive_key(old.as_bytes(), self.header.kdf, &self.header.salt)?;
        let current = self.header.encode();
        crypto::open(
            &kek,
            &self.header.wrap_nonce,
            key_wrap_aad(&current),
            &self.header.wrapped_key,
        )
        .map_err(|()| VaultError::WrongPassword)?;
        let new = normalise(new);
        if new.chars().count() < MIN_PASSWORD_CHARS {
            return Err(VaultError::WeakPassword);
        }
        let mut header = (*self.header).clone();
        // a weaker stored cost is raised to the current target (per field, never lowered)
        let stored = header.kdf;
        let target = self.upgrade_target;
        header.kdf = KdfParams {
            memory_kib: stored.memory_kib.max(target.memory_kib),
            iterations: stored.iterations.max(target.iterations),
            parallelism: stored.parallelism.max(target.parallelism),
        };
        header.kdf.validate()?;
        header.salt = crypto::random_array()?;
        header.wrap_nonce = crypto::random_array()?;
        let kek = crypto::derive_key(new.as_bytes(), header.kdf, &header.salt)?;
        let aad = header.encode();
        let wrapped = crypto::seal(&kek, &header.wrap_nonce, key_wrap_aad(&aad), key.as_bytes())?;
        header.wrapped_key = wrapped
            .try_into()
            .map_err(|_| VaultError::SaveFailed("unexpected wrapped key size".to_owned()))?;
        let previous = std::mem::replace(&mut self.header, Box::new(header));
        let result = self.save();
        if result.is_err() {
            self.header = previous;
        }
        result
    }

    /// Lost-update guard: the file on disk must still start with the header this handle
    /// last read or wrote (the body nonce changes on every save, so the header identifies
    /// the version). A missing file is fine (the save recreates it). A file that cannot
    /// be read as a vault, is a newer format, is shorter than a header or has another
    /// header was written by someone else, and is never overwritten.
    ///
    /// This is a check, then a write, not a lock: a sync tool can still win the race in
    /// the instant between the check and the rename.
    fn check_unchanged_on_disk(&self) -> Result<(), VaultError> {
        // never through a symlink, never a FIFO or device (a save would block on it)
        let file = match fernjo_fs::open_regular(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(VaultError::SaveFailed(e.to_string())),
        };
        let mut head = Vec::with_capacity(HEADER_LEN);
        file.take(HEADER_LEN as u64)
            .read_to_end(&mut head)
            .map_err(|e| VaultError::SaveFailed(e.to_string()))?;
        if head == self.disk_header.as_slice() {
            Ok(())
        } else {
            Err(VaultError::ChangedOnDisk)
        }
    }

    /// Encrypts the body with a fresh nonce and writes the file atomically: temp file in
    /// the same folder, `sync_all`, current file kept as `.bak`, rename. Refuses with
    /// `changed_on_disk` when the file was changed since this handle read or wrote it.
    pub(crate) fn save(&mut self) -> Result<(), VaultError> {
        let unlocked = self.unlocked.as_ref().ok_or(VaultError::VaultLocked)?;
        self.check_unchanged_on_disk()?;
        let generation = unlocked.body.generation.max(self.floor).saturating_add(1);
        let plain = Zeroizing::new(
            serde_json::to_vec(&BodyOut {
                generation,
                entries: &unlocked.body.entries,
            })
            .map_err(|e| VaultError::SaveFailed(e.to_string()))?,
        );
        // The readers refuse a file above MAX_FILE_LEN (a vault that cannot be opened again),
        // so refuse before encrypting or writing anything. The file is the header plus the
        // ciphertext, which is the plaintext plus the authentication tag.
        check_file_len((HEADER_LEN + plain.len() + TAG_LEN) as u64, MAX_FILE_LEN)?;
        let mut header = (*self.header).clone();
        header.body_nonce = crypto::random_array()?;
        let header_bytes = header.encode();
        let ciphertext = crypto::seal(
            unlocked.key.as_bytes(),
            &header.body_nonce,
            &header_bytes,
            &plain,
        )?;
        let mut file = Vec::with_capacity(header_bytes.len() + ciphertext.len());
        file.extend_from_slice(&header_bytes);
        file.extend_from_slice(&ciphertext);
        write_atomic(&self.path, &file).map_err(|e| VaultError::SaveFailed(e.to_string()))?;
        *self.disk_header = header_bytes;
        *self.header = header;
        // memory changes only after the file is written
        if let Some(u) = self.unlocked.as_mut() {
            u.body.generation = generation;
        }
        self.floor = self.floor.max(generation);
        if let Some(store) = &self.generations {
            store.raise(self.header.id, generation);
        }
        Ok(())
    }
}

/// Refuses a vault file of `len` bytes when the readers (`MAX_FILE_LEN`, see `read_capped`)
/// would refuse to open it again.
fn check_file_len(len: u64, max: u64) -> Result<(), VaultError> {
    if len > max {
        return Err(VaultError::SaveFailed(format!(
            "the vault would be larger than {} MiB, the most that can be opened again",
            max / (1024 * 1024)
        )));
    }
    Ok(())
}

fn entry_from(id: EntryId, name: String, new: NewEntry) -> Entry {
    Entry {
        id,
        name,
        username: non_empty(new.username),
        domain: non_empty(new.domain),
        secret: new.secret,
        notes: new.notes,
    }
}

fn checked_name(name: &str) -> Result<String, VaultError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(VaultError::EmptyName);
    }
    Ok(name.to_owned())
}

/// Authenticates and parses the body; any failure is `tampered`.
fn open_body(
    key: &DataKey,
    header: &Header,
    header_bytes: &[u8],
    ciphertext: &[u8],
) -> Result<Body, VaultError> {
    let plain = crypto::open(key.as_bytes(), &header.body_nonce, header_bytes, ciphertext)
        .map_err(|()| VaultError::Tampered)?;
    serde_json::from_slice(&plain).map_err(|_| VaultError::Tampered)
}

/// An entry as the (id, `NewEntry`) pair the backup and sync paths use.
fn export_pair(e: &Entry) -> (EntryId, NewEntry) {
    (
        e.id,
        NewEntry {
            name: e.name.clone(),
            username: e.username.clone(),
            domain: e.domain.clone(),
            secret: e.secret.clone(),
            notes: e.notes.clone(),
        },
    )
}

/// What `inspect_file` learns from the header of a vault file without unlocking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// The vault id from the header.
    pub id: VaultId,
    /// The header bytes (150): they include the body nonce, which is fresh on every save, so
    /// they identify this version of the file.
    pub version_header: Vec<u8>,
    /// The part of the header that belongs to the master password: the KDF cost, the salt and
    /// the wrapped data key (the id, the version and the body nonce are left out). It changes
    /// when the password is changed or the wrap is re-made and stays the same across ordinary
    /// saves, so two files with the same wrap open with the same password.
    pub password_wrap: Vec<u8>,
}

/// Reads the vault id and version identity of file bytes. Uses the same parser as `open`, so
/// the KDF bounds, the newer-format rule and the size cap apply.
///
/// # Errors
/// `unreadable`, `newer_format`, `invalid_kdf_params`.
pub fn inspect_file(bytes: &[u8]) -> Result<FileInfo, VaultError> {
    check_foreign_len(bytes)?;
    let (header, _) = Header::parse(bytes)?;
    let password_wrap = header.password_wrap();
    Ok(FileInfo {
        id: header.id,
        version_header: bytes[..HEADER_LEN].to_vec(),
        password_wrap,
    })
}

/// The decrypted content of a vault file: its generation and entries with secrets.
pub struct Contents {
    pub generation: u64,
    pub entries: Vec<(EntryId, NewEntry)>,
}

impl fmt::Debug for Contents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Contents")
            .field("generation", &self.generation)
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// Opens the body of file bytes with a data key (the password is not needed: the key is stable
/// across password changes). Secret material leaves here; the caller handles it like
/// [`Vault::export_entries`].
///
/// # Errors
/// `unreadable`, `newer_format`, `invalid_kdf_params`, `tampered` (the key does not open it).
pub fn read_entries(bytes: &[u8], key: &DataKey) -> Result<Contents, VaultError> {
    check_foreign_len(bytes)?;
    let (header, ciphertext) = Header::parse(bytes)?;
    let body = open_body(key, &header, &header.encode(), ciphertext)?;
    Ok(Contents {
        generation: body.generation,
        entries: body.entries.iter().map(export_pair).collect(),
    })
}

fn check_foreign_len(bytes: &[u8]) -> Result<(), VaultError> {
    if bytes.len() as u64 > MAX_FILE_LEN {
        return Err(VaultError::Unreadable(
            "the file is larger than the vault size limit".to_owned(),
        ));
    }
    Ok(())
}

/// A temp file of an interrupted save older than this is left over and gets removed by
/// [`Vault::open`] (`fernjo_fs::sweep_stale_temps`, which the other data files share).
pub const STALE_TEMP_AGE: Duration = fernjo_fs::STALE_TEMP_AGE;

/// Writes `bytes` to `path` through `fernjo_fs::write_atomic` with the data-file options:
/// missing folders created private, a random `<file>.<16 hex>.tmp` (the name
/// `fernjo_fs::sweep_stale_temps` looks for), owner-only modes on Unix, the previous file kept as
/// `.bak` (never written through a symlink at that name), rename, folder sync on Unix.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fernjo_fs::write_atomic(path, bytes, fernjo_fs::WriteOptions::data_file())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};

    use secrecy::SecretString;
    use tempfile::TempDir;

    use fernjo_fs::with_suffix;

    use crate::format::{
        HEADER_LEN, MAX_FILE_LEN, OFF_KDF_ITERATIONS, OFF_KDF_MEMORY, OFF_KDF_PARALLELISM,
        OFF_VERSION, read_capped,
    };
    use crate::{DataKey, FILE_NAME, KdfParams, Vault, VaultError};

    const PW: &str = "correct horse battery";

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn new_vault(dir: &TempDir) -> (Vault, PathBuf) {
        let path = dir.path().join(FILE_NAME);
        let vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
        (vault, path)
    }

    fn code<T>(r: Result<T, VaultError>) -> &'static str {
        match r {
            Ok(_) => "ok",
            Err(e) => e.code(),
        }
    }

    fn bak(path: &Path) -> PathBuf {
        let mut name = path.as_os_str().to_owned();
        name.push(".bak");
        PathBuf::from(name)
    }

    #[test]
    fn create_lock_unlock_round_trip() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        assert!(path.exists());
        assert!(vault.is_unlocked());
        vault.lock();
        assert!(!vault.is_unlocked());
        vault.unlock(&secret(PW)).unwrap();
        assert!(vault.is_unlocked());

        let mut reopened = Vault::open(&path).unwrap().unwrap();
        assert!(!reopened.is_unlocked());
        assert_eq!(reopened.id(), vault.id());
        assert_eq!(reopened.path(), path.as_path());
        reopened.unlock(&secret(PW)).unwrap();
        assert!(reopened.is_unlocked());
    }

    #[cfg(unix)]
    #[test]
    fn vault_files_are_owner_only_and_a_new_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("fresh").join("fernjo");
        let path = folder.join(FILE_NAME);
        let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
        assert_eq!(mode(&folder), 0o700, "a folder fernjo creates is private");
        assert_eq!(mode(&path), 0o600);
        vault
            .change_password(&secret(PW), &secret("another long password"))
            .unwrap();
        assert_eq!(mode(&bak(&path)), 0o600);
        assert_eq!(mode(&path), 0o600);

        // a vault an older build wrote with the default umask is tightened by the next save
        // (and so is the backup copied from it)
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        vault
            .change_password(&secret("another long password"), &secret(PW))
            .unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&bak(&path)), 0o600);
    }

    #[test]
    fn open_of_a_missing_file_is_none() {
        let dir = TempDir::new().unwrap();
        assert!(Vault::open(&dir.path().join(FILE_NAME)).unwrap().is_none());
    }

    #[test]
    fn wrong_password_is_rejected_with_wrong_password_code() {
        let dir = TempDir::new().unwrap();
        let (mut vault, _) = new_vault(&dir);
        vault.lock();
        let err = vault.unlock(&secret("not the password")).unwrap_err();
        assert_eq!(err.code(), "wrong_password");
        assert!(!vault.is_unlocked());
    }

    #[test]
    fn short_password_is_rejected_with_weak_password() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let r = Vault::create_with(&path, &secret("1234567"), KdfParams::TEST);
        assert_eq!(code(r), "weak_password");
        assert!(!path.exists());
        let ok = Vault::create_with(&path, &secret("12345678"), KdfParams::TEST);
        assert_eq!(code(ok), "ok");
    }

    #[test]
    fn nfc_and_nfd_passwords_unlock_the_same_vault() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(FILE_NAME);
        let mut vault =
            Vault::create_with(&path, &secret("caf\u{e9}-pass"), KdfParams::TEST).unwrap();
        vault.lock();
        vault.unlock(&secret("cafe\u{301}-pass")).unwrap();
        vault.lock();
        vault.unlock(&secret("caf\u{e9}-pass")).unwrap();
        // no trimming: a trailing space is a different password
        vault.lock();
        assert_eq!(
            code(vault.unlock(&secret("caf\u{e9}-pass "))),
            "wrong_password"
        );
    }

    #[test]
    fn flipping_any_byte_of_header_wrapped_key_or_body_is_rejected() {
        let dir = TempDir::new().unwrap();
        let (vault, path) = new_vault(&dir);
        drop(vault);
        let original = fs::read(&path).unwrap();
        assert!(original.len() > HEADER_LEN);
        for offset in 0..original.len() {
            let mut bytes = original.clone();
            bytes[offset] ^= 0x01;
            fs::write(&path, &bytes).unwrap();
            let outcome = Vault::open(&path).and_then(|v| {
                let mut v = v.expect("file exists");
                v.unlock(&secret(PW))
            });
            let c = code(outcome);
            assert!(
                [
                    "wrong_password",
                    "tampered",
                    "unreadable",
                    "newer_format",
                    "invalid_kdf_params"
                ]
                .contains(&c),
                "offset {offset} gave {c}"
            );
        }
    }

    #[test]
    fn empty_and_truncated_files_are_unreadable_not_a_panic() {
        let dir = TempDir::new().unwrap();
        let (vault, path) = new_vault(&dir);
        drop(vault);
        let original = fs::read(&path).unwrap();
        for bytes in [
            Vec::new(),
            original[..5].to_vec(),
            original[..HEADER_LEN].to_vec(),
            original[..HEADER_LEN - 1].to_vec(),
        ] {
            fs::write(&path, &bytes).unwrap();
            let outcome = Vault::open(&path).and_then(|v| v.unwrap().unlock(&secret(PW)));
            assert_eq!(code(outcome), "unreadable", "len {}", bytes.len());
        }
    }

    #[test]
    fn newer_format_version_is_refused_and_file_untouched() {
        let dir = TempDir::new().unwrap();
        let (vault, path) = new_vault(&dir);
        drop(vault);
        let mut bytes = fs::read(&path).unwrap();
        bytes[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&999u16.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert_eq!(code(Vault::open(&path)), "newer_format");
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert!(!bak(&path).exists());
    }

    #[test]
    fn absurd_kdf_parameters_in_header_are_rejected_before_deriving() {
        let dir = TempDir::new().unwrap();
        let (vault, path) = new_vault(&dir);
        drop(vault);
        let mut bytes = fs::read(&path).unwrap();
        let huge_kib: u32 = 64 * 1024 * 1024; // 64 GiB
        bytes[OFF_KDF_MEMORY..OFF_KDF_MEMORY + 4].copy_from_slice(&huge_kib.to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        let started = std::time::Instant::now();
        assert_eq!(code(Vault::open(&path)), "unreadable");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn load_refuses_a_header_with_an_excessive_kdf_cost() {
        // each of memory, passes and lanes alone above 4 times the shipped default
        let d = KdfParams::DEFAULT;
        let cases: [(usize, u32, &str); 3] = [
            (OFF_KDF_MEMORY, d.memory_kib * 4 + 1, "memory"),
            (OFF_KDF_ITERATIONS, d.iterations * 4 + 1, "passes"),
            (OFF_KDF_PARALLELISM, d.parallelism * 4 + 1, "lanes"),
        ];
        for (offset, value, what) in cases {
            let dir = TempDir::new().unwrap();
            let (vault, path) = new_vault(&dir);
            drop(vault);
            let mut bytes = fs::read(&path).unwrap();
            bytes[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
            fs::write(&path, &bytes).unwrap();
            let err = Vault::open(&path).unwrap_err();
            assert_eq!(err.code(), "unreadable", "{what}");
            assert!(err.to_string().contains(what), "{what}: {err}");
            assert_eq!(fs::read(&path).unwrap(), bytes, "file untouched");
            assert!(!bak(&path).exists());
        }
    }

    #[test]
    fn load_accepts_exactly_four_times_the_default_cost() {
        let d = KdfParams::DEFAULT;
        let dir = TempDir::new().unwrap();
        let (vault, path) = new_vault(&dir);
        drop(vault);
        let mut bytes = fs::read(&path).unwrap();
        bytes[OFF_KDF_MEMORY..OFF_KDF_MEMORY + 4]
            .copy_from_slice(&(d.memory_kib * 4).to_be_bytes());
        bytes[OFF_KDF_ITERATIONS..OFF_KDF_ITERATIONS + 4]
            .copy_from_slice(&(d.iterations * 4).to_be_bytes());
        bytes[OFF_KDF_PARALLELISM..OFF_KDF_PARALLELISM + 4]
            .copy_from_slice(&(d.parallelism * 4).to_be_bytes());
        fs::write(&path, &bytes).unwrap();
        assert!(Vault::open(&path).unwrap().is_some());
    }

    #[test]
    fn save_never_writes_through_the_old_fixed_temp_name() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        let old_tmp = with_suffix(&path, ".tmp");
        fs::write(&old_tmp, b"planted").unwrap();
        vault.save().unwrap();
        assert_eq!(fs::read(&old_tmp).unwrap(), b"planted");
    }

    #[cfg(unix)]
    #[test]
    fn save_never_follows_a_planted_temp_symlink() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim data").unwrap();
        std::os::unix::fs::symlink(&victim, with_suffix(&path, ".tmp")).unwrap();
        vault.save().unwrap();
        assert_eq!(fs::read(&victim).unwrap(), b"victim data");
        // the vault itself is still intact
        let mut reopened = Vault::open(&path).unwrap().unwrap();
        reopened.unlock(&secret(PW)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_vault_bak_is_not_followed() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim data").unwrap();
        // a sync tool (or another local user) planted a link at the backup name
        std::os::unix::fs::symlink(&victim, bak(&path)).unwrap();
        let before = fs::read(&path).unwrap();
        vault.save().unwrap();
        assert_eq!(
            fs::read(&victim).unwrap(),
            b"victim data",
            "target untouched"
        );
        let meta = fs::symlink_metadata(bak(&path)).unwrap();
        assert!(meta.file_type().is_file(), "the link was replaced");
        assert_eq!(fs::read(bak(&path)).unwrap(), before);
        let mut reopened = Vault::open(&path).unwrap().unwrap();
        reopened.unlock(&secret(PW)).unwrap();
    }

    #[test]
    fn change_password_upgrades_a_weak_cost_and_never_lowers_one() {
        let target = KdfParams {
            memory_kib: 64,
            iterations: 2,
            parallelism: 1,
        };
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        vault.upgrade_target = target;
        vault
            .change_password(&secret(PW), &secret("another long one"))
            .unwrap();
        let reopened = Vault::open(&path).unwrap().unwrap();
        assert_eq!(reopened.header.kdf, target);
        let mut reopened = reopened;
        reopened.unlock(&secret("another long one")).unwrap();

        // a stronger stored cost is kept, field by field
        let mut vault = reopened;
        vault.upgrade_target = KdfParams {
            memory_kib: 8,
            iterations: 5,
            parallelism: 1,
        };
        vault
            .change_password(&secret("another long one"), &secret(PW))
            .unwrap();
        let after = Vault::open(&path).unwrap().unwrap();
        assert_eq!(
            after.header.kdf,
            KdfParams {
                memory_kib: 64,
                iterations: 5,
                parallelism: 1
            }
        );
    }

    #[test]
    fn create_refuses_an_existing_file() {
        let dir = TempDir::new().unwrap();
        let (_vault, path) = new_vault(&dir);
        let before = fs::read(&path).unwrap();
        let r = Vault::create_with(&path, &secret(PW), KdfParams::TEST);
        assert_eq!(code(r), "already_exists");
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn save_leaves_no_temp_file_and_keeps_bak() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        assert!(!bak(&path).exists());
        let first = fs::read(&path).unwrap();
        vault.save().unwrap();
        assert_eq!(fs::read(bak(&path)).unwrap(), first);
        assert_ne!(fs::read(&path).unwrap(), first, "fresh body nonce");
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["vault.fernjo", "vault.fernjo.bak"]);
    }

    #[test]
    fn saving_a_locked_vault_is_refused() {
        let dir = TempDir::new().unwrap();
        let (mut vault, _) = new_vault(&dir);
        vault.lock();
        assert_eq!(code(vault.save()), "vault_locked");
    }

    #[test]
    fn debug_output_never_contains_the_data_key_bytes() {
        let key = DataKey::from_bytes([0xAB; 32]);
        let text = format!("{key:?}");
        assert!(text.contains("redacted"));
        assert!(!text.contains("171") && !text.contains("ab, ab") && !text.contains("abab"));
        assert_eq!(key.as_bytes(), &[0xAB; 32]);

        let dir = TempDir::new().unwrap();
        let (vault, _) = new_vault(&dir);
        let shown = format!("{vault:?}");
        assert!(shown.contains("Vault"));
        let real = vault.data_key().unwrap();
        let needle = format!("{:?}", real.as_bytes());
        assert!(!shown.contains(&needle));
    }

    #[test]
    fn read_capped_names_the_cap_in_the_oversize_text_and_keeps_not_found() {
        // the cap itself is tested in fernjo-fs; this guards the vault's visible text
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, [7u8; 10]).unwrap();
        assert_eq!(read_capped(&path, 10).unwrap(), vec![7u8; 10]);
        let err = read_capped(&path, 9).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("larger than"));
        fs::write(&path, []).unwrap();
        assert!(read_capped(&path, 0).unwrap().is_empty());
        let missing = read_capped(&dir.path().join("none"), 10).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn an_oversized_vault_file_is_refused_by_open_unlock_and_unlock_with_key() {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        let key = vault.data_key().unwrap();
        vault.lock();
        // grow the file in place: a valid header followed by zeros up to cap + 1 (cheap, no
        // allocation of the content)
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(MAX_FILE_LEN + 1).unwrap();
        drop(file);
        assert_eq!(code(vault.unlock(&secret(PW))), "unreadable");
        assert_eq!(code(vault.unlock_with_key(&key)), "unreadable");
        assert!(!vault.is_unlocked());
        match Vault::open(&path) {
            Err(VaultError::Unreadable(m)) => assert!(m.contains("larger than"), "{m}"),
            other => panic!("expected unreadable, got {:?}", other.map(|_| ())),
        }
        // refusing never touches the file
        assert_eq!(fs::metadata(&path).unwrap().len(), MAX_FILE_LEN + 1);
    }
}

#[cfg(test)]
mod timing {
    #[test]
    #[ignore = "manual: prints the unlock time of the default KDF parameters"]
    fn default_kdf_timing() {
        let salt = [7u8; 16];
        for p in [crate::KdfParams::DEFAULT] {
            let t = std::time::Instant::now();
            for _ in 0..3 {
                crate::crypto::derive_key(b"correct horse", p, &salt).unwrap();
            }
            println!("{:?}: {:?} per derive", p, t.elapsed() / 3);
        }
    }
}

#[cfg(test)]
#[path = "entry_tests.rs"]
mod entry_tests;

#[cfg(test)]
#[path = "generation_tests.rs"]
mod generation_tests;

#[cfg(test)]
#[path = "adopt_tests.rs"]
mod adopt_tests;
