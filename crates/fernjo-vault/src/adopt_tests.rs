//! Sync support: inspect, read, adopt a file, adopt a merge, sweep stale temp files.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use secrecy::SecretString;
use tempfile::TempDir;

use crate::format::{HEADER_LEN, MAX_FILE_LEN, OFF_KDF_MEMORY};
use crate::{
    FILE_NAME, GenerationStore, KdfParams, MemoryGenerationStore, NewEntry, STALE_TEMP_AGE, Secret,
    Vault, VaultError, inspect_file, read_entries,
};

const PW: &str = "correct horse battery";
const PW2: &str = "another long password";

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

fn new_vault(dir: &TempDir) -> (Vault, PathBuf) {
    let path = dir.path().join(FILE_NAME);
    let vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    (vault, path)
}

/// A second device: a copy of the file in its own folder, opened and unlocked.
fn second_device(path: &Path, dir: &TempDir) -> (Vault, PathBuf) {
    let copy = dir.path().join(FILE_NAME);
    fs::copy(path, &copy).unwrap();
    let mut vault = Vault::open(&copy).unwrap().unwrap();
    vault.unlock(&secret(PW)).unwrap();
    (vault, copy)
}

fn names(vault: &Vault) -> Vec<String> {
    let mut n: Vec<String> = vault.list().unwrap().into_iter().map(|e| e.name).collect();
    n.sort();
    n
}

#[test]
fn inspect_file_reports_the_vault_id_and_changes_with_every_save() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let first = fs::read(&path).unwrap();
    let a = inspect_file(&first).unwrap();
    assert_eq!(a.id, vault.id());
    assert_eq!(a.version_header.len(), HEADER_LEN);
    assert_eq!(a.version_header, first[..HEADER_LEN]);
    vault.add(new_entry("one")).unwrap();
    let b = inspect_file(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(b.id, a.id);
    assert_ne!(b.version_header, a.version_header);
}

#[test]
fn inspect_file_refuses_a_kdf_cost_above_the_cap() {
    let dir = TempDir::new().unwrap();
    let (_vault, path) = new_vault(&dir);
    let mut bytes = fs::read(&path).unwrap();
    let too_much = KdfParams::DEFAULT.memory_kib * 4 + 1;
    bytes[OFF_KDF_MEMORY..OFF_KDF_MEMORY + 4].copy_from_slice(&too_much.to_be_bytes());
    assert_eq!(code(inspect_file(&bytes)), "unreadable");
    assert_eq!(code(inspect_file(b"nope")), "unreadable");
}

#[test]
fn read_entries_with_the_held_key_opens_a_file_after_a_password_change() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    vault.add(new_entry("one")).unwrap();
    let key = vault.data_key().unwrap();
    vault.change_password(&secret(PW), &secret(PW2)).unwrap();
    let contents = read_entries(&fs::read(&path).unwrap(), &key).unwrap();
    assert_eq!(contents.generation, vault.generation().unwrap());
    assert_eq!(contents.entries.len(), 1);
    assert_eq!(contents.entries[0].1.name, "one");
    let wrong = crate::DataKey::from_bytes([9; 32]);
    assert_eq!(
        code(read_entries(&fs::read(&path).unwrap(), &wrong)),
        "tampered"
    );
    assert!(!format!("{contents:?}").contains("pw"));
}

#[test]
fn adopt_file_while_locked_stays_locked_and_the_next_unlock_sees_the_new_entries() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = fs::read(&path).unwrap();
    vault.add(new_entry("one")).unwrap();
    let newer = fs::read(&path).unwrap();
    fs::write(&path, &old).unwrap();

    let id = vault.id();
    let mut handle = Vault::open(&path).unwrap().unwrap();
    let store = Arc::new(MemoryGenerationStore::default());
    handle.attach_generation_store(store.clone());
    handle.adopt_file(&newer).unwrap();
    assert!(!handle.is_unlocked());
    assert_eq!(fs::read(&path).unwrap(), newer);
    assert_eq!(store.get(id), None, "a locked adopt leaves the device mark");
    handle.unlock(&secret(PW)).unwrap();
    assert_eq!(names(&handle), ["one"]);
    assert_eq!(handle.generation(), Some(2));
    assert_eq!(store.get(id), Some(2));
}

#[test]
fn adopt_file_while_unlocked_keeps_the_vault_unlocked_and_the_key_valid() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = fs::read(&path).unwrap();
    let mut other = Vault::open(&path).unwrap().unwrap();
    other.unlock(&secret(PW)).unwrap();
    other.add(new_entry("remote")).unwrap();
    let newer = fs::read(&path).unwrap();
    fs::write(&path, &old).unwrap();

    let key_before = vault.data_key().unwrap();
    vault.adopt_file(&newer).unwrap();
    assert!(vault.is_unlocked());
    assert_eq!(names(&vault), ["remote"]);
    assert_eq!(vault.generation(), Some(2));
    assert_eq!(vault.data_key().unwrap().as_bytes(), key_before.as_bytes());
    // the handle keeps working: its own save follows the adopted file
    vault.add(new_entry("local")).unwrap();
    assert_eq!(vault.generation(), Some(3));
    assert_eq!(names(&vault), ["local", "remote"]);
}

#[test]
fn adopt_file_of_another_vault_is_unreadable_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    vault.add(new_entry("one")).unwrap();
    let other_dir = TempDir::new().unwrap();
    let (_other, other_path) = new_vault(&other_dir);
    let foreign = fs::read(&other_path).unwrap();
    let before = fs::read(&path).unwrap();
    let before_bak = fs::read(bak(&path)).unwrap();

    assert_eq!(code(vault.adopt_file(&foreign)), "unreadable");
    vault.lock();
    assert_eq!(code(vault.adopt_file(&foreign)), "unreadable");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(bak(&path)).unwrap(), before_bak);

    // oversize and newer format are refused too
    let huge = vec![0u8; usize::try_from(MAX_FILE_LEN).unwrap() + 1];
    assert_eq!(code(vault.adopt_file(&huge)), "unreadable");
    let mut newer = before.clone();
    newer[crate::format::OFF_VERSION..crate::format::OFF_VERSION + 2]
        .copy_from_slice(&999u16.to_be_bytes());
    assert_eq!(code(vault.adopt_file(&newer)), "newer_format");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(bak(&path)).unwrap(), before_bak);
}

#[test]
fn adopt_file_refuses_when_the_file_changed_on_disk() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let mut other = Vault::open(&path).unwrap().unwrap();
    other.unlock(&secret(PW)).unwrap();
    other.add(new_entry("one")).unwrap();
    let candidate = fs::read(&path).unwrap();
    other.add(new_entry("two")).unwrap();
    let on_disk = fs::read(&path).unwrap();
    let on_disk_bak = fs::read(bak(&path)).unwrap();

    assert_eq!(code(vault.adopt_file(&candidate)), "changed_on_disk");
    vault.lock();
    assert_eq!(code(vault.adopt_file(&candidate)), "changed_on_disk");
    assert_eq!(fs::read(&path).unwrap(), on_disk);
    assert_eq!(fs::read(bak(&path)).unwrap(), on_disk_bak);
}

#[test]
fn adopt_file_with_a_lower_generation_is_rolled_back_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let store = Arc::new(MemoryGenerationStore::default());
    vault.attach_generation_store(store.clone());
    let first = fs::read(&path).unwrap();
    vault.add(new_entry("one")).unwrap();
    vault.add(new_entry("two")).unwrap();
    let before = fs::read(&path).unwrap();
    let before_bak = fs::read(bak(&path)).unwrap();

    assert_eq!(code(vault.adopt_file(&first)), "rolled_back");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(bak(&path)).unwrap(), before_bak);
    assert_eq!(vault.generation(), Some(3));
    assert_eq!(store.get(vault.id()), Some(3), "the mark is never lowered");
    assert_eq!(names(&vault), ["one", "two"]);
}

#[test]
fn adopt_file_keeps_a_bak_of_the_previous_file() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = fs::read(&path).unwrap();
    vault.add(new_entry("one")).unwrap();
    let newer = fs::read(&path).unwrap();
    fs::write(&path, &old).unwrap();
    let mut handle = Vault::open(&path).unwrap().unwrap();
    handle.adopt_file(&newer).unwrap();
    assert_eq!(fs::read(bak(&path)).unwrap(), old);
    assert_eq!(fs::read(&path).unwrap(), newer);
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn adopt_merged_writes_a_generation_above_both_sides() {
    // head ahead of local
    let dir = TempDir::new().unwrap();
    let (mut local, path) = new_vault(&dir);
    let dir2 = TempDir::new().unwrap();
    let (mut remote, remote_path) = second_device(&path, &dir2);
    for n in ["a", "b", "c"] {
        remote.add(new_entry(n)).unwrap();
    }
    let head = fs::read(&remote_path).unwrap();
    assert_eq!(remote.generation(), Some(4));
    let mut merged = remote.export_entries().unwrap();
    merged.push((uuid::Uuid::new_v4(), new_entry("local-only")));
    local.adopt_merged(&head, merged).unwrap();
    assert_eq!(local.generation(), Some(5));
    assert_eq!(names(&local), ["a", "b", "c", "local-only"]);

    // local ahead of head
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(reopened.generation(), Some(5));
    for n in ["d", "e", "f"] {
        reopened.add(new_entry(n)).unwrap();
    }
    assert_eq!(reopened.generation(), Some(8));
    let entries = reopened.export_entries().unwrap();
    reopened.adopt_merged(&head, entries).unwrap();
    assert_eq!(reopened.generation(), Some(9));
}

#[test]
fn adopt_merged_takes_the_heads_password_wrap() {
    let dir = TempDir::new().unwrap();
    let (mut local, path) = new_vault(&dir);
    let dir2 = TempDir::new().unwrap();
    let (mut remote, remote_path) = second_device(&path, &dir2);
    remote.change_password(&secret(PW), &secret(PW2)).unwrap();
    remote.add(new_entry("r")).unwrap();
    let head = fs::read(&remote_path).unwrap();
    let entries = remote.export_entries().unwrap();

    local.adopt_merged(&head, entries).unwrap();
    assert!(local.is_unlocked());
    let mut fresh = Vault::open(&path).unwrap().unwrap();
    assert_eq!(code(fresh.unlock(&secret(PW))), "wrong_password");
    fresh.unlock(&secret(PW2)).unwrap();
    assert_eq!(names(&fresh), ["r"]);
}

#[test]
fn adopt_merged_refuses_and_writes_nothing_on_bad_input() {
    let dir = TempDir::new().unwrap();
    let (mut local, path) = new_vault(&dir);
    local.add(new_entry("one")).unwrap();
    let before = fs::read(&path).unwrap();
    let before_bak = fs::read(bak(&path)).unwrap();
    let other_dir = TempDir::new().unwrap();
    let (_other, other_path) = new_vault(&other_dir);
    let foreign = fs::read(&other_path).unwrap();
    let floor_before = local.generation();

    assert_eq!(code(local.adopt_merged(&foreign, Vec::new())), "unreadable");
    let id = uuid::Uuid::new_v4();
    let dup = vec![(id, new_entry("x")), (id, new_entry("y"))];
    assert_eq!(code(local.adopt_merged(&before, dup)), "duplicate_entry");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(bak(&path)).unwrap(), before_bak);
    assert_eq!(local.generation(), floor_before);
    assert_eq!(names(&local), ["one"]);

    local.lock();
    assert_eq!(
        code(local.adopt_merged(&before, Vec::new())),
        "vault_locked"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_failed_adopt_merged_restores_header_floor_and_entries_and_a_later_save_works() {
    let dir = TempDir::new().unwrap();
    let (mut local, path) = new_vault(&dir);
    local.add(new_entry("one")).unwrap();
    let original = fs::read(&path).unwrap();
    assert_eq!(local.generation(), Some(2));

    // the head: another device changed the password and added entries (generation 6)
    let dir2 = TempDir::new().unwrap();
    let (mut remote, remote_path) = second_device(&path, &dir2);
    remote.change_password(&secret(PW), &secret(PW2)).unwrap();
    for n in ["r1", "r2", "r3"] {
        remote.add(new_entry(n)).unwrap();
    }
    assert_eq!(remote.generation(), Some(6));
    let head = fs::read(&remote_path).unwrap();
    let merged = remote.export_entries().unwrap();

    // a foreign writer changes the file under this handle, so the merge's save is refused
    let mut other = Vault::open(&path).unwrap().unwrap();
    other.unlock(&secret(PW)).unwrap();
    other.add(new_entry("foreign")).unwrap();
    let on_disk = fs::read(&path).unwrap();
    let on_disk_bak = fs::read(bak(&path)).unwrap();

    assert_eq!(code(local.adopt_merged(&head, merged)), "changed_on_disk");
    assert_eq!(fs::read(&path).unwrap(), on_disk, "the file is untouched");
    assert_eq!(fs::read(bak(&path)).unwrap(), on_disk_bak);
    assert_eq!(local.generation(), Some(2), "the generation is as before");
    assert_eq!(names(&local), ["one"], "the entries are as before");

    // the foreign change goes away (the sync tool puts the old file back): the handle is usable
    // again and writes with its own old header and the old floor, not the head's
    fs::write(&path, &original).unwrap();
    local.add(new_entry("later")).unwrap();
    assert_eq!(
        local.generation(),
        Some(3),
        "the floor of the head (6) was not kept"
    );
    let saved = fs::read(&path).unwrap();
    assert_eq!(
        inspect_file(&saved).unwrap().password_wrap,
        inspect_file(&original).unwrap().password_wrap,
        "the head's password wrap was not kept"
    );
    let mut fresh = Vault::open(&path).unwrap().unwrap();
    assert_eq!(code(fresh.unlock(&secret(PW2))), "wrong_password");
    fresh.unlock(&secret(PW)).unwrap();
    assert_eq!(names(&fresh), ["later", "one"]);
}

#[test]
fn a_locked_adopt_of_an_older_file_is_refused_at_the_next_unlock_by_the_device_mark() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let store = Arc::new(MemoryGenerationStore::default());
    vault.attach_generation_store(store.clone());
    let first = fs::read(&path).unwrap();
    vault.add(new_entry("one")).unwrap();
    vault.add(new_entry("two")).unwrap();
    assert_eq!(store.get(vault.id()), Some(3));
    vault.lock();

    // locked, the generation cannot be read: the older file is taken as it is ...
    vault.adopt_file(&first).unwrap();
    assert_eq!(fs::read(&path).unwrap(), first);
    // ... and the unlock finds out. A wrong password is still just a wrong password.
    assert_eq!(
        code(vault.unlock(&secret("not the password"))),
        "wrong_password"
    );
    let err = vault.unlock(&secret(PW)).unwrap_err();
    assert_eq!(err.code(), "rolled_back");
    assert!(!vault.is_unlocked());
    assert_eq!(store.get(vault.id()), Some(3), "the mark is never lowered");
    // the explicit choice of the user opens it and re-stamps the file above the mark
    vault.unlock_accepting_older(&secret(PW)).unwrap();
    assert!(vault.generation().unwrap() > 3);
    assert_eq!(names(&vault), Vec::<String>::new());
}

#[test]
fn password_wrap_is_stable_across_saves_and_changes_with_the_password() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let wrap_of = |path: &Path| {
        inspect_file(&fs::read(path).unwrap())
            .unwrap()
            .password_wrap
    };
    let first = wrap_of(&path);
    assert_eq!(
        first.len(),
        12 + 16 + 24 + 48,
        "cost, salt, nonce, wrapped key"
    );
    vault.add(new_entry("one")).unwrap();
    assert_eq!(wrap_of(&path), first, "an ordinary save keeps the wrap");
    vault.change_password(&secret(PW), &secret(PW2)).unwrap();
    assert_ne!(wrap_of(&path), first, "a password change re-wraps");
    // the version header differs on every save, the wrap does not say anything about the entries
    let before = inspect_file(&fs::read(&path).unwrap()).unwrap();
    vault.add(new_entry("two")).unwrap();
    let after = inspect_file(&fs::read(&path).unwrap()).unwrap();
    assert_ne!(before.version_header, after.version_header);
    assert_eq!(before.password_wrap, after.password_wrap);
}

fn set_age(path: &Path, age: Duration) {
    let file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

#[test]
fn open_removes_vault_temp_files_older_than_a_day_and_keeps_newer_and_foreign_files() {
    let dir = TempDir::new().unwrap();
    let (vault, path) = new_vault(&dir);
    drop(vault);
    let old = STALE_TEMP_AGE + Duration::from_secs(3600);
    let stale = dir.path().join("vault.fernjo.0123456789abcdef.tmp");
    let fresh = dir.path().join("vault.fernjo.fedcba9876543210.tmp");
    let foreign = dir.path().join("connections.json.0123456789abcdef.tmp");
    let upper = dir.path().join("vault.fernjo.ABCDEF0123456789.tmp");
    let short = dir.path().join("vault.fernjo.0123.tmp");
    let plain = dir.path().join("vault.fernjo.tmp");
    let dir_named = dir.path().join("vault.fernjo.aaaaaaaaaaaaaaaa.tmp");
    for p in [&stale, &fresh, &foreign, &upper, &short, &plain] {
        fs::write(p, b"x").unwrap();
    }
    fs::create_dir(&dir_named).unwrap();
    for p in [&stale, &foreign, &upper, &short, &plain] {
        set_age(p, old);
    }
    set_age(&fresh, Duration::from_secs(60));
    let before = fs::read(&path).unwrap();

    Vault::open(&path).unwrap().unwrap();

    assert!(!stale.exists(), "stale temp file removed");
    for p in [&fresh, &foreign, &upper, &short, &plain] {
        assert!(p.exists(), "{p:?} must stay");
    }
    assert!(dir_named.is_dir());
    assert_eq!(fs::read(&path).unwrap(), before);
}

/// Another vault in its own folder: its file bytes, its password and its path.
fn other_vault_file(entries: &[&str]) -> (Vec<u8>, Vault, TempDir) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let mut vault = Vault::create_with(&path, &secret(PW2), KdfParams::TEST).unwrap();
    for n in entries {
        vault.add(new_entry(n)).unwrap();
    }
    (fs::read(&path).unwrap(), vault, dir)
}

#[test]
fn replace_with_file_takes_another_vault_and_ends_locked() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    vault.add(new_entry("mine")).unwrap();
    let store = Arc::new(MemoryGenerationStore::default());
    vault.attach_generation_store(store.clone());
    let old_id = vault.id();
    let before = fs::read(&path).unwrap();
    let (foreign, other, _other_dir) = other_vault_file(&["theirs"]);

    vault.replace_with_file(&foreign).unwrap();

    assert_eq!(fs::read(&path).unwrap(), foreign);
    assert_eq!(fs::read(bak(&path)).unwrap(), before);
    assert_eq!(vault.id(), other.id());
    assert_ne!(vault.id(), old_id);
    assert!(!vault.is_unlocked());
    assert_eq!(store.get(old_id), Some(2), "the old vault's mark stays");
    // the handle follows the new file: the new password opens it, the old one does not
    assert_eq!(code(vault.unlock(&secret(PW))), "wrong_password");
    vault.unlock(&secret(PW2)).unwrap();
    assert_eq!(names(&vault), ["theirs"]);
    vault.add(new_entry("more")).unwrap();
    assert_eq!(names(&vault), ["more", "theirs"]);
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn replace_with_file_uses_the_device_mark_of_the_new_vault_and_never_writes_it() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _path) = new_vault(&dir);
    let (foreign, other, _other_dir) = other_vault_file(&[]);
    let store = Arc::new(MemoryGenerationStore::default());
    // this device has seen a newer generation of that vault before
    store.raise(other.id(), 9);
    vault.attach_generation_store(store.clone());

    vault.replace_with_file(&foreign).unwrap();
    assert_eq!(store.get(other.id()), Some(9), "the mark is not lowered");
    assert_eq!(code(vault.unlock(&secret(PW2))), "rolled_back");
    assert!(!vault.is_unlocked());
}

#[test]
fn replace_with_file_of_the_same_vault_ends_locked_too() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = fs::read(&path).unwrap();
    vault.add(new_entry("one")).unwrap();
    let newer = fs::read(&path).unwrap();
    fs::write(&path, &old).unwrap();
    let mut handle = Vault::open(&path).unwrap().unwrap();
    handle.unlock(&secret(PW)).unwrap();

    handle.replace_with_file(&newer).unwrap();
    assert!(!handle.is_unlocked());
    handle.unlock(&secret(PW)).unwrap();
    assert_eq!(names(&handle), ["one"]);
}

#[test]
fn replace_with_file_refuses_bad_bytes_and_a_changed_file_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    vault.add(new_entry("one")).unwrap();
    let before = fs::read(&path).unwrap();
    let before_bak = fs::read(bak(&path)).unwrap();
    let (foreign, _other, _other_dir) = other_vault_file(&[]);

    let huge = vec![0u8; usize::try_from(MAX_FILE_LEN).unwrap() + 1];
    assert_eq!(code(vault.replace_with_file(&huge)), "unreadable");
    assert_eq!(code(vault.replace_with_file(b"nope")), "unreadable");
    let mut newer = foreign.clone();
    newer[crate::format::OFF_VERSION..crate::format::OFF_VERSION + 2]
        .copy_from_slice(&999u16.to_be_bytes());
    assert_eq!(code(vault.replace_with_file(&newer)), "newer_format");
    let mut heavy = foreign.clone();
    let too_much = KdfParams::DEFAULT.memory_kib * 4 + 1;
    heavy[OFF_KDF_MEMORY..OFF_KDF_MEMORY + 4].copy_from_slice(&too_much.to_be_bytes());
    assert_eq!(code(vault.replace_with_file(&heavy)), "unreadable");
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(bak(&path)).unwrap(), before_bak);
    assert!(vault.is_unlocked(), "a refused replace changes nothing");

    // somebody else wrote the file since this handle read it
    let mut other = Vault::open(&path).unwrap().unwrap();
    other.unlock(&secret(PW)).unwrap();
    other.add(new_entry("two")).unwrap();
    let on_disk = fs::read(&path).unwrap();
    assert_eq!(code(vault.replace_with_file(&foreign)), "changed_on_disk");
    assert_eq!(fs::read(&path).unwrap(), on_disk);
}

#[test]
fn install_file_creates_a_locked_vault_and_never_overwrites() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(FILE_NAME);
    let (foreign, other, _other_dir) = other_vault_file(&["theirs"]);

    let mut vault = Vault::install_file(&path, &foreign).unwrap();
    assert_eq!(fs::read(&path).unwrap(), foreign);
    assert_eq!(vault.id(), other.id());
    assert!(!vault.is_unlocked());
    vault.unlock(&secret(PW2)).unwrap();
    assert_eq!(names(&vault), ["theirs"]);

    // an existing file, usable or not, is never replaced
    assert_eq!(code(Vault::install_file(&path, &foreign)), "already_exists");
    fs::write(&path, b"garbage").unwrap();
    assert_eq!(code(Vault::install_file(&path, &foreign)), "already_exists");
    assert_eq!(fs::read(&path).unwrap(), b"garbage");

    // bad bytes write nothing
    let fresh = dir.path().join("second.fernjo");
    assert_eq!(code(Vault::install_file(&fresh, b"nope")), "unreadable");
    let huge = vec![0u8; usize::try_from(MAX_FILE_LEN).unwrap() + 1];
    assert_eq!(code(Vault::install_file(&fresh, &huge)), "unreadable");
    assert!(!fresh.exists());
}
