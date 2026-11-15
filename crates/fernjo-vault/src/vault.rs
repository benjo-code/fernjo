//! The `Vault`: create, open, unlock, lock, and the encrypted atomic save.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::crypto::{self, DataKey, NONCE_LEN, SALT_LEN};
use crate::entry::{Entry, EntryId, EntrySummary, EntryUpdate, NewEntry, Secret, non_empty};
use crate::error::VaultError;
use crate::format::{Header, KdfParams, OFF_WRAPPED_KEY, WRAPPED_KEY_LEN};

/// Identifier of a vault, stored in its header; keys the OS keychain item (Task 3).
pub type VaultId = Uuid;

/// Minimum master password length in characters, counted after NFC normalisation.
const MIN_PASSWORD_CHARS: usize = 8;

/// The decrypted body: `{ "entries": [...] }`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Body {
    #[serde(default)]
    pub entries: Vec<Entry>,
}

struct Unlocked {
    key: DataKey,
    body: Body,
}

/// An encrypted vault file. Locked vaults hold no key and no decrypted entries.
pub struct Vault {
    path: PathBuf,
    header: Header,
    unlocked: Option<Unlocked>,
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
fn normalise(password: &SecretString) -> Zeroizing<String> {
    Zeroizing::new(password.expose_secret().nfc().collect::<String>())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
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
            header,
            unlocked: Some(Unlocked {
                key,
                body: Body::default(),
            }),
        };
        vault.save()?;
        Ok(vault)
    }

    /// Opens an existing vault file, locked. `Ok(None)` if the file does not exist.
    ///
    /// # Errors
    /// `unreadable`, `newer_format`, `invalid_kdf_params`.
    pub fn open(path: &Path) -> Result<Option<Self>, VaultError> {
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(VaultError::Unreadable(e.to_string())),
        };
        let (header, _) = Header::parse(&bytes)?;
        Ok(Some(Self {
            path: path.to_path_buf(),
            header,
            unlocked: None,
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

    /// Unlocks with the master password. The file is read again, so a vault updated on
    /// disk by a sync tool is picked up; a file with another vault id is refused. On any
    /// error the vault stays locked.
    ///
    /// # Errors
    /// `wrong_password`, `tampered`, `unreadable`, `newer_format`, `invalid_kdf_params`.
    pub fn unlock(&mut self, password: &SecretString) -> Result<(), VaultError> {
        let bytes = fs::read(&self.path).map_err(|e| VaultError::Unreadable(e.to_string()))?;
        let (header, ciphertext) = Header::parse(&bytes)?;
        if header.id != self.header.id {
            return Err(VaultError::Unreadable(
                "the vault file was replaced by a different vault".to_owned(),
            ));
        }
        let password = normalise(password);
        let kek = crypto::derive_key(password.as_bytes(), header.kdf, &header.salt)?;
        let header_bytes = header.encode();
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
        let key = DataKey::from_bytes(key_bytes);
        let body = open_body(&key, &header, &header_bytes, ciphertext)?;
        self.header = header;
        self.unlocked = Some(Unlocked { key, body });
        Ok(())
    }

    /// Unlocks with the data key (for example read from the OS keychain). Same file
    /// handling as [`Vault::unlock`]; a key that does not open the body is refused.
    ///
    /// # Errors
    /// `tampered` (the key does not fit the file), `unreadable`, `newer_format`,
    /// `invalid_kdf_params`.
    pub fn unlock_with_key(&mut self, key: &DataKey) -> Result<(), VaultError> {
        let bytes = fs::read(&self.path).map_err(|e| VaultError::Unreadable(e.to_string()))?;
        let (header, ciphertext) = Header::parse(&bytes)?;
        if header.id != self.header.id {
            return Err(VaultError::Unreadable(
                "the vault file was replaced by a different vault".to_owned(),
            ));
        }
        let header_bytes = header.encode();
        let body = open_body(key, &header, &header_bytes, ciphertext)?;
        self.header = header;
        self.unlocked = Some(Unlocked {
            key: DataKey::from_bytes(*key.as_bytes()),
            body,
        });
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
    /// `vault_locked`, `empty_name`, `save_failed` (memory unchanged).
    pub fn add(&mut self, new: NewEntry) -> Result<EntryId, VaultError> {
        let name = checked_name(&new.name)?;
        let id = Uuid::new_v4();
        self.mutate(|entries| {
            entries.push(Entry {
                id,
                name,
                username: non_empty(new.username),
                domain: non_empty(new.domain),
                secret: new.secret,
                notes: new.notes,
            });
            Ok(())
        })?;
        Ok(id)
    }

    /// Applies a partial update and saves.
    ///
    /// # Errors
    /// `vault_locked`, `not_found`, `empty_name`, `save_failed` (memory unchanged).
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
    /// `vault_locked`, `not_found`, `save_failed` (memory unchanged).
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
    /// under the same data key with a fresh nonce, because the header is its AAD). A
    /// failure leaves the file and the header in memory as they were.
    ///
    /// # Errors
    /// `vault_locked`, `wrong_password`, `weak_password`, `save_failed`.
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
        let mut header = self.header.clone();
        header.salt = crypto::random_array()?;
        header.wrap_nonce = crypto::random_array()?;
        let kek = crypto::derive_key(new.as_bytes(), header.kdf, &header.salt)?;
        let aad = header.encode();
        let wrapped = crypto::seal(&kek, &header.wrap_nonce, key_wrap_aad(&aad), key.as_bytes())?;
        header.wrapped_key = wrapped
            .try_into()
            .map_err(|_| VaultError::SaveFailed("unexpected wrapped key size".to_owned()))?;
        let previous = std::mem::replace(&mut self.header, header);
        let result = self.save();
        if result.is_err() {
            self.header = previous;
        }
        result
    }

    /// Encrypts the body with a fresh nonce and writes the file atomically: temp file in
    /// the same folder, `sync_all`, current file copied to `.bak`, rename.
    pub(crate) fn save(&mut self) -> Result<(), VaultError> {
        let unlocked = self.unlocked.as_ref().ok_or(VaultError::VaultLocked)?;
        let plain = Zeroizing::new(
            serde_json::to_vec(&unlocked.body)
                .map_err(|e| VaultError::SaveFailed(e.to_string()))?,
        );
        let mut header = self.header.clone();
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
        self.header = header;
        Ok(())
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

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(dir)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let result = (|| -> io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            fs::copy(path, with_suffix(path, ".bak"))?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use secrecy::SecretString;
    use tempfile::TempDir;

    use crate::format::{HEADER_LEN, OFF_KDF_MEMORY, OFF_VERSION};
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
        assert_eq!(code(Vault::open(&path)), "invalid_kdf_params");
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
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
