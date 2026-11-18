//! Tests for the key store trait and auto-unlock (child module of `keystore`).

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

/// Wraps a [`MemoryKeyStore`] and counts the `delete` calls.
#[derive(Default)]
struct CountingStore {
    inner: MemoryKeyStore,
    deletes: std::sync::atomic::AtomicUsize,
}

impl CountingStore {
    fn deletes(&self) -> usize {
        self.deletes.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl KeyStore for CountingStore {
    fn get(&self, id: VaultId) -> Result<Option<DataKey>, KeyStoreError> {
        self.inner.get(id)
    }
    fn set(&self, id: VaultId, key: &DataKey) -> Result<(), KeyStoreError> {
        self.inner.set(id, key)
    }
    fn delete(&self, id: VaultId) -> Result<(), KeyStoreError> {
        self.deletes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.delete(id)
    }
}

/// A locked vault whose real key is in a counting store.
fn locked_with_key(dir: &TempDir) -> (Vault, CountingStore) {
    let mut vault = new_vault(dir);
    let store = CountingStore::default();
    store.set(vault.id(), &vault.data_key().unwrap()).unwrap();
    vault.lock();
    (vault, store)
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
fn wrong_stored_key_is_not_accepted_and_the_item_is_kept() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    let store = CountingStore::default();
    let wrong = DataKey::from_bytes([7u8; 32]);
    store.set(vault.id(), &wrong).unwrap();
    assert_eq!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::KeyNotAccepted
    );
    assert!(!vault.is_unlocked());
    assert_eq!(store.deletes(), 0);
    let kept = store.get(vault.id()).unwrap().expect("item kept");
    assert_eq!(kept.as_bytes(), wrong.as_bytes());
    vault.unlock(&secret(PW)).unwrap();
    assert!(vault.is_unlocked());
}

#[test]
fn a_bit_flipped_body_keeps_the_item_and_the_restored_file_unlocks_again() {
    let dir = TempDir::new().unwrap();
    let (mut vault, store) = locked_with_key(&dir);
    let good = std::fs::read(vault.path()).unwrap();
    let mut bad = good.clone();
    *bad.last_mut().unwrap() ^= 0x01;
    std::fs::write(vault.path(), &bad).unwrap();
    assert_eq!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::KeyNotAccepted
    );
    assert!(!vault.is_unlocked());
    assert_eq!(store.deletes(), 0);
    assert!(store.get(vault.id()).unwrap().is_some());
    std::fs::write(vault.path(), &good).unwrap();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::Unlocked);
    assert!(vault.is_unlocked());
}

#[test]
fn a_truncated_file_keeps_the_item() {
    let dir = TempDir::new().unwrap();
    let (mut vault, store) = locked_with_key(&dir);
    let good = std::fs::read(vault.path()).unwrap();
    std::fs::write(vault.path(), &good[..20]).unwrap();
    assert!(matches!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::Unavailable(_)
    ));
    assert!(!vault.is_unlocked());
    assert_eq!(store.deletes(), 0);
    assert!(store.get(vault.id()).unwrap().is_some());
    std::fs::write(vault.path(), &good).unwrap();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::Unlocked);
}

#[test]
fn a_planted_same_id_vault_with_another_key_keeps_the_item() {
    let dir = TempDir::new().unwrap();
    let (mut vault, store) = locked_with_key(&dir);
    // another vault (other data key) whose file carries this vault's id
    let other_dir = TempDir::new().unwrap();
    let other = new_vault(&other_dir);
    let mut planted = std::fs::read(other.path()).unwrap();
    let at = crate::format::OFF_ID;
    planted[at..at + 16].copy_from_slice(vault.id().as_bytes());
    std::fs::write(vault.path(), &planted).unwrap();
    assert_eq!(
        try_auto_unlock(&mut vault, &store),
        AutoUnlock::KeyNotAccepted
    );
    assert!(!vault.is_unlocked());
    assert_eq!(store.deletes(), 0);
    assert!(store.get(vault.id()).unwrap().is_some());
}

#[test]
fn a_different_id_file_finds_no_key_and_never_touches_the_old_item() {
    let dir = TempDir::new().unwrap();
    let (old, store) = locked_with_key(&dir);
    let other_dir = TempDir::new().unwrap();
    let mut replacement = new_vault(&other_dir);
    replacement.lock();
    assert_eq!(try_auto_unlock(&mut replacement, &store), AutoUnlock::NoKey);
    assert_eq!(store.deletes(), 0);
    assert!(store.get(old.id()).unwrap().is_some());
}

#[test]
fn a_password_change_copied_from_another_device_still_auto_unlocks() {
    let dir = TempDir::new().unwrap();
    let (mut vault, store) = locked_with_key(&dir);
    // "another device" changes the password on its own copy of the file
    let device = TempDir::new().unwrap();
    std::fs::copy(vault.path(), device.path().join(FILE_NAME)).unwrap();
    let mut elsewhere = Vault::open(&device.path().join(FILE_NAME))
        .unwrap()
        .unwrap();
    elsewhere.unlock(&secret(PW)).unwrap();
    elsewhere
        .change_password(&secret(PW), &secret(NEW_PW))
        .unwrap();
    std::fs::copy(elsewhere.path(), vault.path()).unwrap();
    assert_eq!(try_auto_unlock(&mut vault, &store), AutoUnlock::Unlocked);
    assert_eq!(store.deletes(), 0);
}

#[test]
fn store_failure_stays_locked_with_keychain_unavailable() {
    let dir = TempDir::new().unwrap();
    let mut vault = new_vault(&dir);
    vault.lock();
    assert_eq!(
        try_auto_unlock(&mut vault, &FailingStore),
        AutoUnlock::KeychainUnavailable("keychain unavailable".to_owned())
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
