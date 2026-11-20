//! Rollback protection of the vault: the generation counter in the body and the device marks.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use secrecy::SecretString;
use tempfile::TempDir;

use crate::{
    DataKey, EntryUpdate, FILE_NAME, GenerationStore, KdfParams, MemoryGenerationStore, NewEntry,
    Secret, Vault, VaultError,
};

const PW: &str = "correct horse battery";

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn bak(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

fn new_entry(name: &str) -> NewEntry {
    NewEntry {
        name: name.to_owned(),
        username: None,
        domain: None,
        secret: Secret::Password {
            password: secret("pw"),
        },
        notes: String::new(),
    }
}

fn code<T>(r: Result<T, VaultError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

fn generation(v: &Vault) -> u64 {
    v.generation().unwrap()
}

/// A new vault with a memory store attached.
fn new_vault(dir: &TempDir) -> (Vault, PathBuf, Arc<MemoryGenerationStore>) {
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    let store = Arc::new(MemoryGenerationStore::default());
    vault.attach_generation_store(store.clone());
    (vault, path, store)
}

/// Opens the file as a fresh handle (as after a restart) with `store` attached.
fn reopen(path: &Path, store: Arc<dyn GenerationStore>) -> Vault {
    let mut vault = Vault::open(path).unwrap().unwrap();
    vault.attach_generation_store(store);
    vault
}

/// Adds entries until the vault is at `target` and returns the file bytes of each generation.
fn climb(vault: &mut Vault, path: &Path, target: u64) -> Vec<Vec<u8>> {
    let mut files = vec![fs::read(path).unwrap()]; // generation 1 (create)
    while generation(vault) < target {
        vault.add(new_entry("e")).unwrap();
        files.push(fs::read(path).unwrap());
    }
    files
}

/// Puts `bytes` in place of the vault file without any handle noticing.
fn put_back(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
}

#[test]
fn generation_is_one_after_create_and_grows_with_every_save() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _path, store) = new_vault(&dir);
    assert_eq!(generation(&vault), 1);
    assert_eq!(store.get(vault.id()), Some(1));
    let id = vault.add(new_entry("a")).unwrap();
    assert_eq!(generation(&vault), 2);
    vault
        .update(
            id,
            EntryUpdate {
                notes: Some("n".to_owned()),
                ..EntryUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(generation(&vault), 3);
    vault.delete(id).unwrap();
    assert_eq!(generation(&vault), 4);
    vault
        .change_password(&secret(PW), &secret("another long password"))
        .unwrap();
    assert_eq!(generation(&vault), 5, "a password change counts too");
    vault.replace_entries(vec![]).unwrap();
    assert_eq!(generation(&vault), 6, "a restore counts too");
    assert_eq!(store.get(vault.id()), Some(6));
    vault.lock();
    assert_eq!(vault.generation(), None, "locked");
}

#[test]
fn a_failed_save_leaves_generation_floor_and_mark_untouched() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, store) = new_vault(&dir);
    vault.add(new_entry("a")).unwrap();
    assert_eq!(generation(&vault), 2);
    // somebody else changes the file: the save guard refuses
    let mut bytes = fs::read(&path).unwrap();
    bytes[40] ^= 1;
    put_back(&path, &bytes);
    assert_eq!(code(vault.add(new_entry("b"))), "changed_on_disk");
    assert_eq!(generation(&vault), 2);
    assert_eq!(store.get(vault.id()), Some(2));
    assert_eq!(vault.list().unwrap().len(), 1);
}

#[test]
fn an_older_file_is_refused_after_the_password_proved_genuine() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, _store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    assert_eq!(generation(&vault), 3);
    // the sync tool puts the previous version (.bak, generation 2) back
    let older = fs::read(bak(&path)).unwrap();
    put_back(&path, &older);
    vault.lock();
    let err = vault.unlock(&secret(PW)).unwrap_err();
    assert!(matches!(err, VaultError::RolledBack { found: 2, seen: 3 }));
    assert_eq!(err.code(), "rolled_back");
    assert_eq!(
        err.to_string(),
        "The vault file is older than the version this device last opened (generation 2, last seen 3). Another device or a sync tool may have put an older copy back."
    );
    assert!(!vault.is_unlocked());
    assert_eq!(
        fs::read(&path).unwrap(),
        older,
        "refusing never touches the file"
    );
}

#[test]
fn a_wrong_password_never_reveals_a_rollback() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, _store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    vault.lock();
    assert_eq!(
        code(vault.unlock(&secret("not the password"))),
        "wrong_password"
    );
    assert_eq!(
        code(vault.unlock_with_key(&DataKey::from_bytes([9; 32]))),
        "tampered"
    );
    assert_eq!(
        code(vault.unlock_accepting_older(&secret("not the password"))),
        "wrong_password"
    );
    assert!(!vault.is_unlocked());
}

#[test]
fn unlock_with_key_is_refused_the_same_way() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, _store) = new_vault(&dir);
    let key = vault.data_key().unwrap();
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    vault.lock();
    let err = vault.unlock_with_key(&key).unwrap_err();
    assert!(matches!(err, VaultError::RolledBack { found: 2, seen: 3 }));
    assert!(!vault.is_unlocked());
}

#[test]
fn the_floor_survives_a_restart_through_the_store_only() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    drop(vault);
    // a new handle with the same store refuses; one with an empty store has no way to know
    let mut protected = reopen(&path, store.clone());
    assert_eq!(code(protected.unlock(&secret(PW))), "rolled_back");
    let mut blind = reopen(&path, Arc::new(MemoryGenerationStore::default()));
    blind.unlock(&secret(PW)).unwrap();
    assert_eq!(generation(&blind), 2);
}

#[test]
fn accepting_the_older_file_restamps_it_above_the_floor() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    let older = fs::read(bak(&path)).unwrap();
    put_back(&path, &older);
    vault.lock();
    assert_eq!(code(vault.unlock(&secret(PW))), "rolled_back");

    vault.unlock_accepting_older(&secret(PW)).unwrap();
    assert!(vault.is_unlocked());
    assert_eq!(
        generation(&vault),
        4,
        "floor 3 + 1: accepting does not lower the floor"
    );
    assert_eq!(store.get(vault.id()), Some(4));
    assert_eq!(vault.list().unwrap().len(), 1, "the older content");
    assert_eq!(
        fs::read(bak(&path)).unwrap(),
        older,
        ".bak holds the pre-accept file"
    );

    // another device that had seen generation 3 accepts the re-stamped file without a prompt
    let other = Arc::new(MemoryGenerationStore::default());
    other.raise(vault.id(), 3);
    let mut second = reopen(&path, other.clone());
    second.unlock(&secret(PW)).unwrap();
    assert_eq!(generation(&second), 4);
    assert_eq!(other.get(second.id()), Some(4));
    // and a normal unlock on this device works again
    vault.lock();
    vault.unlock(&secret(PW)).unwrap();
}

#[test]
fn a_failed_restamp_keeps_the_unlock_and_the_next_save_still_goes_above_the_floor() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    vault.lock();
    // make the re-stamp fail: the `.bak` name is taken by a directory (the temp file name is
    // random, so it cannot be blocked)
    let tmp = bak(&path).into_os_string();
    fs::remove_file(PathBuf::from(&tmp)).unwrap();
    fs::create_dir(PathBuf::from(&tmp)).unwrap();
    vault.unlock_accepting_older(&secret(PW)).unwrap();
    assert!(vault.is_unlocked());
    assert_eq!(generation(&vault), 2, "not re-stamped");
    assert_eq!(store.get(vault.id()), Some(3), "the floor was not lowered");
    // the prompt returns on the next start
    let mut again = reopen(&path, store.clone());
    assert_eq!(code(again.unlock(&secret(PW))), "rolled_back");
    // a later save writes floor + 1
    fs::remove_dir(PathBuf::from(&tmp)).unwrap();
    vault.add(new_entry("later")).unwrap();
    assert_eq!(generation(&vault), 4);
}

#[test]
fn unlock_accepting_older_on_a_current_file_is_a_plain_unlock() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, _store) = new_vault(&dir);
    vault.lock();
    let before = fs::read(&path).unwrap();
    vault.unlock_accepting_older(&secret(PW)).unwrap();
    assert_eq!(generation(&vault), 1);
    assert_eq!(fs::read(&path).unwrap(), before, "no re-stamp needed");
}

#[test]
fn an_equal_generation_is_accepted() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, store) = new_vault(&dir);
    climb(&mut vault, &path, 3);
    vault.lock();
    vault.unlock(&secret(PW)).unwrap();
    assert_eq!(generation(&vault), 3);
    let mut fresh = reopen(&path, store);
    fresh.unlock(&secret(PW)).unwrap();
}

#[test]
fn a_newer_generation_from_another_device_opens_silently_and_raises_the_mark() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path, _store) = new_vault(&dir);
    climb(&mut vault, &path, 5);
    let this_device = Arc::new(MemoryGenerationStore::default());
    this_device.raise(vault.id(), 2);
    let mut here = reopen(&path, this_device.clone());
    here.unlock(&secret(PW)).unwrap();
    assert_eq!(generation(&here), 5);
    assert_eq!(this_device.get(here.id()), Some(5));
}

#[test]
fn marks_are_per_vault_id() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();
    let shared = Arc::new(MemoryGenerationStore::default());
    let mut a =
        Vault::create_with(&dir_a.path().join(FILE_NAME), &secret(PW), KdfParams::TEST).unwrap();
    a.attach_generation_store(shared.clone());
    let path_a = dir_a.path().join(FILE_NAME);
    climb(&mut a, &path_a, 6);
    // a vault with another id and a low generation is not judged against A's mark
    let mut b =
        Vault::create_with(&dir_b.path().join(FILE_NAME), &secret(PW), KdfParams::TEST).unwrap();
    b.attach_generation_store(shared.clone());
    b.lock();
    b.unlock(&secret(PW)).unwrap();
    assert_eq!(shared.get(a.id()), Some(6));
    assert_eq!(shared.get(b.id()), Some(1));
}

/// A store that knows nothing and remembers nothing.
struct BrokenStore;

impl GenerationStore for BrokenStore {
    fn get(&self, _vault: crate::VaultId) -> Option<u64> {
        None
    }
    fn raise(&self, _vault: crate::VaultId, _generation: u64) {}
}

#[test]
fn an_unreadable_store_gives_no_protection_but_the_in_memory_floor_still_does() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    vault.attach_generation_store(Arc::new(BrokenStore));
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    vault.lock();
    assert_eq!(
        code(vault.unlock(&secret(PW))),
        "rolled_back",
        "the handle remembers what it saw"
    );
    // a fresh handle with the same broken store cannot know, and must not fail
    let mut fresh = reopen(&path, Arc::new(BrokenStore));
    fresh.unlock(&secret(PW)).unwrap();
}

#[test]
fn without_any_store_the_floor_still_protects_lock_unlock_cycles() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    climb(&mut vault, &path, 3);
    put_back(&path, &fs::read(bak(&path)).unwrap());
    vault.lock();
    assert_eq!(code(vault.unlock(&secret(PW))), "rolled_back");
}

#[test]
fn two_devices_saving_in_turn_conflict_case() {
    // device A saved generations 6 and 7; the sync delivers device B's older generation 6
    let dir = TempDir::new().unwrap();
    let (mut a, path, store_a) = new_vault(&dir);
    let files = climb(&mut a, &path, 7);
    let gen6 = files[5].clone();
    assert_eq!(store_a.get(a.id()), Some(7));
    put_back(&path, &gen6);
    a.lock();
    let err = a.unlock(&secret(PW)).unwrap_err();
    assert!(matches!(err, VaultError::RolledBack { found: 6, seen: 7 }));
    a.unlock_accepting_older(&secret(PW)).unwrap();
    assert_eq!(generation(&a), 8);
}

#[test]
fn attaching_a_store_to_an_unlocked_vault_raises_the_mark_and_takes_the_floor() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    climb(&mut vault, &path, 3);
    let store = Arc::new(MemoryGenerationStore::default());
    vault.attach_generation_store(store.clone());
    assert_eq!(store.get(vault.id()), Some(3));

    let high = Arc::new(MemoryGenerationStore::default());
    high.raise(vault.id(), 10);
    vault.lock();
    vault.attach_generation_store(high);
    assert_eq!(code(vault.unlock(&secret(PW))), "rolled_back");
}

#[test]
fn a_saved_vault_never_writes_a_lower_generation_than_its_floor() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    let store = Arc::new(MemoryGenerationStore::default());
    store.raise(vault.id(), 20);
    vault.attach_generation_store(store.clone());
    vault.add(new_entry("a")).unwrap();
    assert_eq!(generation(&vault), 21);
    assert_eq!(store.get(vault.id()), Some(21));
}
