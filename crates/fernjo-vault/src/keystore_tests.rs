//! Tests for the key store trait and auto-unlock (child module of `keystore`).

use std::collections::HashMap;
use std::sync::Mutex;

use secrecy::SecretString;
use tempfile::TempDir;
use uuid::Uuid;

use super::*;
use crate::{DataKey, FILE_NAME, KdfParams, Vault, VaultId};

const PW: &str = "correct horse battery";
const NEW_PW: &str = "a brand new password";

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn new_vault(dir: &TempDir) -> Vault {
    Vault::create_with(&dir.path().join(FILE_NAME), &secret(PW), KdfParams::TEST).unwrap()
}

/// A key store whose every call fails.
struct FailingStore;

impl KeyStore for FailingStore {
    fn get(&self, _: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
        Err(KeyStoreError::new("keychain unavailable"))
    }
    fn set(&self, _: VaultId, _: &DataKey) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::new("keychain unavailable"))
    }
    fn delete(&self, _: VaultId) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::new("keychain unavailable"))
    }
}

/// Returns a key for `get`, but refuses to delete it.
struct StuckStore(Mutex<HashMap<VaultId, [u8; 32]>>);

impl KeyStore for StuckStore {
    fn get(&self, id: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&id)
            .map(|b| DataKey::from_bytes(*b)))
    }
    fn set(&self, _: VaultId, _: &DataKey) -> Result<(), KeyStoreError> {
        Ok(())
    }
    fn delete(&self, _: VaultId) -> Result<(), KeyStoreError> {
        Err(KeyStoreError::new("cannot delete"))
    }
}

#[test]
fn auto_unlock_unlocks_when_key_is_stored() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    let store = MemoryKeyStore::default();
    store.set(vault.id(), &vault.data_key().unwrap()).unwrap();
    vault.lock();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::Unlocked);
    assert!(vault.is_unlocked());
}

#[test]
fn no_stored_key_stays_locked_with_no_key() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    let store = MemoryKeyStore::default();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::NoKey);
    assert!(!vault.is_unlocked());
}

#[test]
fn wrong_stored_key_stays_locked_and_the_item_is_deleted() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    let store = MemoryKeyStore::default();
    store
        .set(vault.id(), &DataKey::from_bytes([7u8; 32]))
        .unwrap();
    assert_eq!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::StaleKeyRemoved
    );
    assert!(!vault.is_unlocked());
    assert!(store.get(vault.id()).unwrap().is_none());
    vault.unlock(&secret(PW)).unwrap();
    assert!(vault.is_unlocked());
}

#[test]
fn a_failing_delete_of_a_stale_key_does_not_panic_and_stays_locked() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    let store = StuckStore(Mutex::new(HashMap::from([(vault.id(), [7u8; 32])])));
    assert_eq!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::StaleKeyRemoved
    );
    assert!(!vault.is_unlocked());
}

#[test]
fn store_failure_stays_locked_with_unavailable() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    assert_eq!(
        try_auto_unlock(&mut vault, &FailingStore),
        AutoUnlock::Unavailable("keychain unavailable".to_owned())
    );
    assert!(!vault.is_unlocked());
}

#[test]
fn other_unlock_errors_are_unavailable_and_keep_the_item() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    let store = MemoryKeyStore::default();
    store.set(vault.id(), &vault.data_key().unwrap()).unwrap();
    vault.lock();
    std::fs::remove_file(vault.path()).unwrap();
    assert!(matches!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::Unavailable(_)
    ));
    assert!(!vault.is_unlocked());
    assert!(store.get(vault.id()).unwrap().is_some());
}

#[test]
fn keychain_key_stays_valid_after_change_password() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    let store = MemoryKeyStore::default();
    store.set(vault.id(), &vault.data_key().unwrap()).unwrap();
    vault.change_password(&secret(PW), &secret(NEW_PW)).unwrap();
    vault.lock();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::Unlocked);
    assert!(vault.is_unlocked());
}

#[test]
fn keys_are_per_vault_id() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let a = new_vault(&dir_a);
    let mut b = new_vault(&dir_b);
    let store = MemoryKeyStore::default();
    store.set(a.id(), &a.data_key().unwrap()).unwrap();
    b.lock();
    assert_ne!(a.id(), b.id());
    assert!(store.get(b.id()).unwrap().is_none());
    assert_eq!(try_auto_unlock(&mut b, &store), AutoUnlock::NoKey);
}

#[test]
fn key_store_error_and_debug_never_show_key_bytes() {
    let key = DataKey::from_bytes([0xAB; 32]);
    assert!(!format!("{key:?}").contains("171"));
    let store = MemoryKeyStore::default();
    store.set(Uuid::new_v4(), &key).unwrap();
    assert!(!format!("{store:?}").contains("171"));
}

/// Manual check against the real OS keychain (random id, deleted again):
/// `cargo test -p fernjo-vault os_keystore_round_trip -- --ignored`
#[test]
#[ignore = "touches the real OS keychain"]
fn os_keystore_round_trip() {
    let store = OsKeyStore;
    let id = Uuid::new_v4();
    let key = DataKey::from_bytes([0x5A; 32]);
    assert!(store.get(id).unwrap().is_none());
    store.set(id, &key).unwrap();
    let back = store.get(id).unwrap().expect("key present");
    assert_eq!(back.as_bytes(), key.as_bytes());
    store.delete(id).unwrap();
    assert!(store.get(id).unwrap().is_none());
    // Deleting a missing item is not an error.
    store.delete(id).unwrap();
}
