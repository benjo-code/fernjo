//! OS keychain storage of the vault data key and the auto-unlock built on it.
//!
//! Threat model: with auto-unlock on, the data key sits in the OS keychain, so anything
//! running as the same OS user can open the vault. See `Docs/DECISIONS.md`
//! ("Vault: keychain behaviour").

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use zeroize::Zeroizing;

use crate::crypto::KEY_LEN;
use crate::{DataKey, Vault, VaultError, VaultId};

/// Keychain service name of all fernjo items.
const SERVICE: &str = "fernjo";

/// A keychain failure with a message fit for display. Never contains key bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyStoreError(String);

impl KeyStoreError {
    /// Creates an error from a display message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The display message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KeyStoreError {}

/// Storage for one data key per vault id.
pub trait KeyStore: Send + Sync {
    /// The stored key, or `None` when there is no item.
    ///
    /// # Errors
    /// The store is unavailable or the item is unusable.
    fn get(&self, vault: VaultId) -> Result<Option<DataKey>, KeyStoreError>;

    /// Stores (or replaces) the key.
    ///
    /// # Errors
    /// The store is unavailable.
    fn set(&self, vault: VaultId, key: &DataKey) -> Result<(), KeyStoreError>;

    /// Removes the key; a missing item is not an error.
    ///
    /// # Errors
    /// The store is unavailable.
    fn delete(&self, vault: VaultId) -> Result<(), KeyStoreError>;
}

/// In-memory key store for tests.
#[derive(Default)]
pub struct MemoryKeyStore {
    keys: Mutex<HashMap<VaultId, Zeroizing<[u8; KEY_LEN]>>>,
}

impl fmt::Debug for MemoryKeyStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemoryKeyStore(<redacted>)")
    }
}

impl MemoryKeyStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<VaultId, Zeroizing<[u8; KEY_LEN]>>> {
        self.keys
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, vault: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
        Ok(self.lock().get(&vault).map(|b| DataKey::from_bytes(**b)))
    }

    fn set(&self, vault: VaultId, key: &DataKey) -> Result<(), KeyStoreError> {
        self.lock().insert(vault, Zeroizing::new(*key.as_bytes()));
        Ok(())
    }

    fn delete(&self, vault: VaultId) -> Result<(), KeyStoreError> {
        self.lock().remove(&vault);
        Ok(())
    }
}

/// The OS keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux)
/// through the `keyring` crate. Service `fernjo`, user = the vault id, secret = the key.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsKeyStore;

fn map_err(e: &keyring::Error) -> KeyStoreError {
    // `keyring::Error` messages describe the platform failure, never the secret.
    KeyStoreError::new(format!("OS keychain: {e}"))
}

fn entry(vault: VaultId) -> Result<keyring::Entry, KeyStoreError> {
    keyring::Entry::new(SERVICE, &vault.to_string()).map_err(|e| map_err(&e))
}

impl KeyStore for OsKeyStore {
    fn get(&self, vault: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
        let secret = match entry(vault)?.get_secret() {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(e) => return Err(map_err(&e)),
        };
        let bytes: [u8; KEY_LEN] = secret
            .as_slice()
            .try_into()
            .map_err(|_| KeyStoreError::new("OS keychain: the stored item has the wrong size"))?;
        let key = DataKey::from_bytes(bytes);
        Ok(Some(key))
    }

    fn set(&self, vault: VaultId, key: &DataKey) -> Result<(), KeyStoreError> {
        entry(vault)?
            .set_secret(key.as_bytes())
            .map_err(|e| map_err(&e))
    }

    fn delete(&self, vault: VaultId) -> Result<(), KeyStoreError> {
        match entry(vault)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_err(&e)),
        }
    }
}

/// Result of [`try_auto_unlock`]. The vault stays locked in every case but `Unlocked`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoUnlock {
    /// The stored key opened the vault.
    Unlocked,
    /// No key is stored; ask for the master password.
    NoKey,
    /// The file did not open with the stored key (wrong item, or a damaged, half-synced or
    /// replaced file). The item is kept; ask for the master password.
    KeyNotAccepted,
    /// The keychain could not be read (no Secret Service, locked keyring, access denied), so
    /// nothing is known about a stored key; ask for the master password.
    KeychainUnavailable(String),
    /// The vault file could not be used (unreadable, truncated, replaced, newer format); the
    /// item is kept; ask for the master password.
    Unavailable(String),
    /// The stored key opened the file, but the file is older than the version this device last
    /// opened (a sync tool may have put an older copy back). Auto-unlock never accepts that;
    /// the item is kept and the user decides with the master password.
    RolledBack { found: u64, seen: u64 },
}

/// Tries to unlock `vault` with the key from `store`. Never prompts, never panics and never
/// deletes the item: the key never changes for a vault id (not even on a password change),
/// and the file may be damaged or half-synced right now, so a failure proves nothing about
/// the item. Only the user's explicit "off" removes it.
pub fn try_auto_unlock(vault: &mut Vault, store: &dyn KeyStore) -> AutoUnlock {
    let key = match store.get(vault.id()) {
        Ok(Some(key)) => key,
        Ok(None) => return AutoUnlock::NoKey,
        Err(e) => return AutoUnlock::KeychainUnavailable(e.0),
    };
    match vault.unlock_with_key(&key) {
        Ok(()) => AutoUnlock::Unlocked,
        Err(VaultError::RolledBack { found, seen }) => AutoUnlock::RolledBack { found, seen },
        Err(e) if e.code() == "tampered" => AutoUnlock::KeyNotAccepted,
        Err(e) => AutoUnlock::Unavailable(e.to_string()),
    }
}

#[cfg(test)]
#[path = "keystore_tests.rs"]
mod tests;
