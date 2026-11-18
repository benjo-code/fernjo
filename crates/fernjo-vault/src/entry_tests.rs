//! Tests for entries and the master password change (child module of `vault`).

use std::fs;
use std::path::PathBuf;

use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

use crate::format::{OFF_BODY_NONCE, OFF_SALT, OFF_WRAP_NONCE};
use crate::{EntryUpdate, FILE_NAME, KdfParams, NewEntry, Secret, SecretKind, Vault, VaultError};

const PW: &str = "correct horse battery";
const NEW_PW: &str = "a brand new password";
const SENTINEL: &str = "SENTINEL-secret-9f3a";

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn new_vault(dir: &TempDir) -> (Vault, PathBuf) {
    let path = dir.path().join(FILE_NAME);
    let vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    (vault, path)
}

fn password_entry(name: &str, pw: &str) -> NewEntry {
    NewEntry {
        name: name.to_owned(),
        username: Some("root".to_owned()),
        domain: Some("EXAMPLE".to_owned()),
        secret: Secret::Password {
            password: secret(pw),
        },
        notes: "line1\nline2".to_owned(),
    }
}

fn key_entry(name: &str, key: &str, pass: Option<&str>) -> NewEntry {
    NewEntry {
        name: name.to_owned(),
        username: None,
        domain: None,
        secret: Secret::PrivateKey {
            key: secret(key),
            passphrase: pass.map(secret),
        },
        notes: String::new(),
    }
}

fn rename(name: &str) -> EntryUpdate {
    EntryUpdate {
        name: Some(name.to_owned()),
        ..EntryUpdate::default()
    }
}

fn code<T>(r: Result<T, VaultError>) -> &'static str {
    match r {
        Ok(_) => "ok",
        Err(e) => e.code(),
    }
}

fn password_of(s: &Secret) -> String {
    match s {
        Secret::Password { password } => password.expose_secret().to_owned(),
        Secret::PrivateKey { .. } => panic!("not a password"),
    }
}

fn key_of(s: &Secret) -> (String, Option<String>) {
    match s {
        Secret::PrivateKey { key, passphrase } => (
            key.expose_secret().to_owned(),
            passphrase.as_ref().map(|p| p.expose_secret().to_owned()),
        ),
        Secret::Password { .. } => panic!("not a key"),
    }
}

#[test]
fn entries_survive_lock_and_reopen() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let a = vault.add(password_entry("web", "pw-one")).unwrap();
    let b = vault
        .add(key_entry("git", "KEYDATA", Some("pass-two")))
        .unwrap();
    drop(vault);

    let mut vault = Vault::open(&path).unwrap().unwrap();
    assert_eq!(code(vault.list()), "vault_locked");
    vault.unlock(&secret(PW)).unwrap();
    assert_eq!(vault.list().unwrap().len(), 2);
    let sa = vault.get(a).unwrap();
    assert_eq!(sa.name, "web");
    assert_eq!(sa.kind, SecretKind::Password);
    assert_eq!(sa.username.as_deref(), Some("root"));
    assert_eq!(sa.domain.as_deref(), Some("EXAMPLE"));
    assert_eq!(sa.notes, "line1\nline2");
    assert_eq!(password_of(&vault.reveal(a).unwrap()), "pw-one");
    let sb = vault.get(b).unwrap();
    assert_eq!(sb.kind, SecretKind::PrivateKey);
    assert_eq!(sb.username, None);
    assert_eq!(
        key_of(&vault.reveal(b).unwrap()),
        ("KEYDATA".to_owned(), Some("pass-two".to_owned()))
    );
}

#[test]
fn update_without_secret_keeps_the_stored_secret() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let id = vault.add(password_entry("old", "keep-me")).unwrap();
    vault
        .update(
            id,
            EntryUpdate {
                name: Some("new".to_owned()),
                username: Some(String::new()),
                ..EntryUpdate::default()
            },
        )
        .unwrap();
    let s = vault.get(id).unwrap();
    assert_eq!(s.name, "new");
    assert_eq!(s.username, None, "empty string clears");
    assert_eq!(s.domain.as_deref(), Some("EXAMPLE"), "None keeps");
    assert_eq!(password_of(&vault.reveal(id).unwrap()), "keep-me");

    vault
        .update(
            id,
            EntryUpdate {
                secret: Some(Secret::Password {
                    password: secret("replaced"),
                }),
                ..EntryUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(password_of(&vault.reveal(id).unwrap()), "replaced");
    assert_eq!(code(vault.update(id, rename("  "))), "empty_name");
    assert_eq!(vault.get(id).unwrap().name, "new");
}

#[test]
fn secrets_are_stored_byte_exact() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let pw = "  pa ss  ";
    let key = "-----BEGIN KEY-----\r\nAAAA\r\n-----END KEY-----";
    let a = vault.add(password_entry("p", pw)).unwrap();
    let b = vault.add(key_entry("k", key, Some(" pp\t"))).unwrap();
    drop(vault);
    let mut vault = Vault::open(&path).unwrap().unwrap();
    vault.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&vault.reveal(a).unwrap()), pw);
    assert_eq!(
        key_of(&vault.reveal(b).unwrap()),
        (key.to_owned(), Some(" pp\t".to_owned()))
    );
}

#[test]
fn list_and_get_expose_no_secret() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let a = vault.add(password_entry("p", SENTINEL)).unwrap();
    vault.add(key_entry("k", SENTINEL, Some(SENTINEL))).unwrap();
    let json = serde_json::to_string(&vault.list().unwrap()).unwrap();
    assert!(!json.contains(SENTINEL));
    assert!(json.contains("\"kind\":\"privateKey\""));
    assert!(json.contains("\"kind\":\"password\""));
    let one = serde_json::to_string(&vault.get(a).unwrap()).unwrap();
    assert!(!one.contains(SENTINEL));
}

#[test]
fn debug_output_never_contains_a_secret() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let a = vault.add(password_entry("p", SENTINEL)).unwrap();
    vault.add(key_entry("k", SENTINEL, Some(SENTINEL))).unwrap();
    assert!(!format!("{vault:?}").contains(SENTINEL));
    let revealed = vault.reveal(a).unwrap();
    assert!(!format!("{revealed:?}").contains(SENTINEL));
    let body = &vault.unlocked.as_ref().unwrap().body;
    assert!(!format!("{body:?}").contains(SENTINEL));
    let new = key_entry("k", SENTINEL, Some(SENTINEL));
    assert!(!format!("{new:?}").contains(SENTINEL));
}

#[test]
fn locked_vault_refuses_every_entry_operation_with_vault_locked() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let id = vault.add(password_entry("p", "pw")).unwrap();
    vault.lock();
    assert_eq!(code(vault.add(password_entry("x", "y"))), "vault_locked");
    assert_eq!(
        code(vault.update(id, EntryUpdate::default())),
        "vault_locked"
    );
    assert_eq!(code(vault.delete(id)), "vault_locked");
    assert_eq!(code(vault.list()), "vault_locked");
    assert_eq!(code(vault.get(id)), "vault_locked");
    assert_eq!(code(vault.reveal(id)), "vault_locked");
    assert_eq!(code(vault.data_key()), "vault_locked");
    assert_eq!(
        code(vault.change_password(&secret(PW), &secret(NEW_PW))),
        "vault_locked"
    );
}

#[test]
fn delete_unknown_entry_is_not_found() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let id = vault.add(password_entry("p", "pw")).unwrap();
    let other = uuid::Uuid::new_v4();
    assert_eq!(code(vault.delete(other)), "not_found");
    assert_eq!(code(vault.get(other)), "not_found");
    assert_eq!(code(vault.reveal(other)), "not_found");
    assert_eq!(
        code(vault.update(other, EntryUpdate::default())),
        "not_found"
    );
    vault.delete(id).unwrap();
    assert!(vault.list().unwrap().is_empty());
    assert_eq!(code(vault.delete(id)), "not_found");
}

#[test]
fn empty_name_is_rejected() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    assert_eq!(code(vault.add(password_entry("", "pw"))), "empty_name");
    assert_eq!(code(vault.add(password_entry("   ", "pw"))), "empty_name");
    assert!(vault.list().unwrap().is_empty());
}

/// Creates a vault in `dir/sub`, then replaces the folder by a file so saves fail.
fn vault_with_broken_target(dir: &TempDir) -> (Vault, PathBuf, PathBuf) {
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let path = sub.join(FILE_NAME);
    let vault = Vault::create_with(&path, &secret(PW), KdfParams::TEST).unwrap();
    (vault, sub, path)
}

fn break_target(sub: &PathBuf) {
    fs::remove_dir_all(sub).unwrap();
    fs::write(sub, b"not a folder").unwrap();
}

#[test]
fn failed_save_leaves_entries_unchanged() {
    let dir = TempDir::new().unwrap();
    let (mut vault, sub, _) = vault_with_broken_target(&dir);
    let keep = vault.add(password_entry("keep", "pw")).unwrap();
    break_target(&sub);

    assert_eq!(code(vault.add(password_entry("new", "pw"))), "save_failed");
    assert_eq!(code(vault.delete(keep)), "save_failed");
    assert_eq!(code(vault.update(keep, rename("renamed"))), "save_failed");
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "keep");
    assert_eq!(password_of(&vault.reveal(keep).unwrap()), "pw");
}

fn header_slices(path: &PathBuf) -> ([u8; 16], [u8; 24], [u8; 24]) {
    let b = fs::read(path).unwrap();
    (
        b[OFF_SALT..OFF_SALT + 16].try_into().unwrap(),
        b[OFF_WRAP_NONCE..OFF_WRAP_NONCE + 24].try_into().unwrap(),
        b[OFF_BODY_NONCE..OFF_BODY_NONCE + 24].try_into().unwrap(),
    )
}

#[test]
fn change_password_keeps_entries_and_data_key() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let a = vault.add(password_entry("p", "pw-one")).unwrap();
    let b = vault.add(key_entry("k", "KEY\r\n", Some("pp"))).unwrap();
    let old_key = vault.data_key().unwrap();
    let (salt0, wrap0, body0) = header_slices(&path);

    vault.change_password(&secret(PW), &secret(NEW_PW)).unwrap();
    let (salt1, wrap1, body1) = header_slices(&path);
    assert_ne!(salt0, salt1);
    assert_ne!(wrap0, wrap1);
    assert_ne!(body0, body1);
    assert_eq!(vault.data_key().unwrap().as_bytes(), old_key.as_bytes());

    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock_with_key(&old_key).unwrap();
    assert_eq!(password_of(&reopened.reveal(a).unwrap()), "pw-one");
    reopened.lock();
    assert_eq!(code(reopened.unlock(&secret(PW))), "wrong_password");
    reopened.unlock(&secret(NEW_PW)).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 2);
    assert_eq!(
        key_of(&reopened.reveal(b).unwrap()),
        ("KEY\r\n".to_owned(), Some("pp".to_owned()))
    );
}

#[test]
fn change_password_with_wrong_old_password_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let a = vault.add(password_entry("p", "pw-one")).unwrap();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        code(vault.change_password(&secret("wrong password!"), &secret(NEW_PW))),
        "wrong_password"
    );
    assert_eq!(
        code(vault.change_password(&secret(PW), &secret("short"))),
        "weak_password"
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    vault.lock();
    vault.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&vault.reveal(a).unwrap()), "pw-one");
}

#[test]
fn failed_save_during_change_password_keeps_the_old_password_working() {
    let dir = TempDir::new().unwrap();
    let (mut vault, sub, path) = vault_with_broken_target(&dir);
    let first = vault.add(password_entry("first", "pw-one")).unwrap();
    let bytes = fs::read(&path).unwrap();
    break_target(&sub);
    assert_eq!(
        code(vault.change_password(&secret(PW), &secret(NEW_PW))),
        "save_failed"
    );
    // make saving work again, then save from the SAME unlocked vault: that uses the
    // in-memory header, which must still be the old one
    fs::remove_file(&sub).unwrap();
    fs::create_dir(&sub).unwrap();
    fs::write(&path, &bytes).unwrap();
    let second = vault.add(password_entry("second", "pw-two")).unwrap();
    drop(vault);

    let mut reopened = Vault::open(&path).unwrap().unwrap();
    assert_eq!(code(reopened.unlock(&secret(NEW_PW))), "wrong_password");
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&reopened.reveal(first).unwrap()), "pw-one");
    assert_eq!(password_of(&reopened.reveal(second).unwrap()), "pw-two");
}

// --- lost-update guard: a file changed on disk after unlock is never overwritten ---

/// A second handle on the same file, unlocked: stands for another device or process.
fn second_handle(path: &std::path::Path) -> Vault {
    let mut other = Vault::open(path).unwrap().unwrap();
    other.unlock(&secret(PW)).unwrap();
    other
}

#[test]
fn a_save_after_another_writer_changed_the_file_is_refused() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let keep = vault.add(password_entry("keep", "pw-one")).unwrap();
    let mut other = second_handle(&path);
    other
        .add(password_entry("from-elsewhere", "pw-two"))
        .unwrap();
    let on_disk = fs::read(&path).unwrap();

    assert_eq!(
        code(vault.add(password_entry("late", "pw-three"))),
        "changed_on_disk"
    );
    assert_eq!(
        code(vault.update(keep, rename("renamed"))),
        "changed_on_disk"
    );
    assert_eq!(code(vault.delete(keep)), "changed_on_disk");
    assert_eq!(
        code(vault.change_password(&secret(PW), &secret(NEW_PW))),
        "changed_on_disk"
    );
    assert_eq!(fs::read(&path).unwrap(), on_disk, "file untouched");

    // memory is restored, and locking and unlocking picks the other writer's state up
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "keep");
    vault.lock();
    vault.unlock(&secret(PW)).unwrap();
    assert_eq!(vault.list().unwrap().len(), 2);
    vault.add(password_entry("now fine", "pw-four")).unwrap();
    assert_eq!(vault.list().unwrap().len(), 3);
}

#[test]
fn a_save_after_a_password_change_on_another_copy_is_refused() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let mut other = second_handle(&path);
    other.change_password(&secret(PW), &secret(NEW_PW)).unwrap();
    let on_disk = fs::read(&path).unwrap();

    assert_eq!(
        code(vault.add(password_entry("late", "pw"))),
        "changed_on_disk"
    );
    assert_eq!(fs::read(&path).unwrap(), on_disk);
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(NEW_PW)).unwrap();
    assert!(reopened.list().unwrap().is_empty());
}

#[test]
fn a_newer_format_swapped_in_after_unlock_is_not_overwritten() {
    use crate::format::OFF_VERSION;
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let mut bytes = fs::read(&path).unwrap();
    bytes[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&999u16.to_be_bytes());
    fs::write(&path, &bytes).unwrap();

    assert_eq!(
        code(vault.add(password_entry("late", "pw"))),
        "changed_on_disk"
    );
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert!(!dir.path().join("vault.fernjo.bak").exists());
}

#[test]
fn a_garbage_or_empty_file_swapped_in_after_unlock_is_not_overwritten() {
    for garbage in [Vec::new(), b"not a vault".to_vec(), vec![0u8; 400]] {
        let dir = TempDir::new().unwrap();
        let (mut vault, path) = new_vault(&dir);
        fs::write(&path, &garbage).unwrap();
        assert_eq!(
            code(vault.add(password_entry("late", "pw"))),
            "changed_on_disk",
            "len {}",
            garbage.len()
        );
        assert_eq!(fs::read(&path).unwrap(), garbage);
    }
}

#[test]
fn a_header_only_file_is_not_overwritten() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let bytes = fs::read(&path).unwrap();
    let short = bytes[..crate::format::HEADER_LEN - 1].to_vec();
    fs::write(&path, &short).unwrap();
    assert_eq!(
        code(vault.add(password_entry("late", "pw"))),
        "changed_on_disk"
    );
    assert_eq!(fs::read(&path).unwrap(), short);
}

#[test]
fn a_vault_file_deleted_after_unlock_is_recreated_by_the_next_save() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    vault.add(password_entry("one", "pw-one")).unwrap();
    fs::remove_file(&path).unwrap();
    let two = vault.add(password_entry("two", "pw-two")).unwrap();

    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 2);
    assert_eq!(password_of(&reopened.reveal(two).unwrap()), "pw-two");
}

#[test]
fn consecutive_saves_by_the_same_handle_are_not_refused() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let a = vault.add(password_entry("a", "pw")).unwrap();
    vault.update(a, rename("b")).unwrap();
    vault.change_password(&secret(PW), &secret(NEW_PW)).unwrap();
    vault.delete(a).unwrap();
    // a locked-and-unlocked handle follows the file
    vault.lock();
    vault.unlock(&secret(NEW_PW)).unwrap();
    vault.add(password_entry("c", "pw")).unwrap();
}

// --- backup primitives: add with id, export and replace ---

#[test]
fn add_with_id_keeps_the_id_and_rejects_a_duplicate() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let id = uuid::Uuid::from_u128(42);
    vault
        .add_with_id(id, password_entry("fixed", "pw"))
        .unwrap();
    assert_eq!(vault.get(id).unwrap().name, "fixed");
    assert_eq!(
        code(vault.add_with_id(id, password_entry("again", "pw2"))),
        "duplicate_entry"
    );
    assert_eq!(vault.list().unwrap().len(), 1);
    drop(vault);
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&reopened.reveal(id).unwrap()), "pw");
}

#[test]
fn export_entries_requires_unlocked() {
    let dir = TempDir::new().unwrap();
    let (mut vault, _) = new_vault(&dir);
    let a = vault.add(password_entry("web", "pw-one")).unwrap();
    let b = vault.add(key_entry("git", "KEYDATA", Some("pp"))).unwrap();
    let exported = vault.export_entries().unwrap();
    assert_eq!(exported.len(), 2);
    let (_, web) = exported.iter().find(|(id, _)| *id == a).unwrap();
    assert_eq!(web.name, "web");
    assert_eq!(password_of(&web.secret), "pw-one");
    let (_, git) = exported.iter().find(|(id, _)| *id == b).unwrap();
    assert_eq!(
        key_of(&git.secret),
        ("KEYDATA".to_owned(), Some("pp".to_owned()))
    );
    vault.lock();
    assert_eq!(code(vault.export_entries()), "vault_locked");
}

#[test]
fn replace_entries_swaps_all_in_one_save_and_keeps_bak() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = vault.add(password_entry("old", "pw-old")).unwrap();
    let before = fs::read(&path).unwrap();
    let n1 = uuid::Uuid::from_u128(1);
    let n2 = uuid::Uuid::from_u128(2);
    vault
        .replace_entries(vec![
            (n1, password_entry("one", "pw1")),
            (n2, key_entry("two", "K2", None)),
        ])
        .unwrap();
    assert_eq!(code(vault.get(old)), "not_found");
    assert_eq!(vault.list().unwrap().len(), 2);
    let bak = fs::read(path.with_file_name(format!("{FILE_NAME}.bak"))).unwrap();
    assert_eq!(
        bak, before,
        "exactly one save: .bak is the pre-replace file"
    );
    drop(vault);
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&reopened.reveal(n1).unwrap()), "pw1");
    assert_eq!(key_of(&reopened.reveal(n2).unwrap()).0, "K2");
}

#[test]
fn replace_entries_with_duplicate_ids_changes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let keep = vault.add(password_entry("keep", "pw")).unwrap();
    let before = fs::read(&path).unwrap();
    let id = uuid::Uuid::from_u128(9);
    assert_eq!(
        code(vault.replace_entries(vec![
            (id, password_entry("a", "x")),
            (id, password_entry("b", "y")),
        ])),
        "duplicate_entry"
    );
    assert_eq!(vault.list().unwrap().len(), 1);
    assert_eq!(password_of(&vault.reveal(keep).unwrap()), "pw");
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn failed_save_in_replace_entries_keeps_old_entries() {
    let dir = TempDir::new().unwrap();
    let (mut vault, sub, _) = vault_with_broken_target(&dir);
    let keep = vault.add(password_entry("keep", "pw")).unwrap();
    break_target(&sub);
    assert_eq!(
        code(vault.replace_entries(vec![(
            uuid::Uuid::from_u128(5),
            password_entry("new", "pw2")
        )])),
        "save_failed"
    );
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, keep);
}

// --- many entries in one save ---

#[test]
fn add_entries_with_ids_adds_all_in_one_save_and_bak_is_the_pre_import_file() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let old = vault.add(password_entry("old", "pw-old")).unwrap();
    let before = fs::read(&path).unwrap();
    let n1 = uuid::Uuid::from_u128(1);
    let n2 = uuid::Uuid::from_u128(2);
    let n3 = uuid::Uuid::from_u128(3);
    vault
        .add_entries_with_ids(vec![
            (n1, password_entry("one", "pw1")),
            (n2, key_entry("two", "K2", None)),
            (n3, password_entry("three", "pw3")),
        ])
        .unwrap();
    assert_eq!(vault.list().unwrap().len(), 4);
    assert_eq!(password_of(&vault.reveal(old).unwrap()), "pw-old");
    let bak = fs::read(path.with_file_name(format!("{FILE_NAME}.bak"))).unwrap();
    assert_eq!(bak, before, "exactly one save: .bak is the pre-add file");
    drop(vault);
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(password_of(&reopened.reveal(n3).unwrap()), "pw3");
    assert_eq!(key_of(&reopened.reveal(n2).unwrap()).0, "K2");
}

#[test]
fn add_entries_with_ids_validates_before_changing_anything() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let existing = vault.add(password_entry("keep", "pw")).unwrap();
    let before = fs::read(&path).unwrap();
    let a = uuid::Uuid::from_u128(7);
    // repeated id inside the batch
    assert_eq!(
        code(vault.add_entries_with_ids(vec![
            (a, password_entry("a", "x")),
            (a, password_entry("b", "y")),
        ])),
        "duplicate_entry"
    );
    // an id the vault already holds
    assert_eq!(
        code(vault.add_entries_with_ids(vec![
            (uuid::Uuid::from_u128(8), password_entry("c", "x")),
            (existing, password_entry("d", "y")),
        ])),
        "duplicate_entry"
    );
    // an empty name
    assert_eq!(
        code(vault.add_entries_with_ids(vec![
            (uuid::Uuid::from_u128(9), password_entry("e", "x")),
            (uuid::Uuid::from_u128(10), password_entry("  ", "y")),
        ])),
        "empty_name"
    );
    assert_eq!(vault.list().unwrap().len(), 1);
    assert_eq!(fs::read(&path).unwrap(), before, "nothing was written");
    vault.lock();
    assert_eq!(
        code(vault.add_entries_with_ids(vec![(a, password_entry("a", "x"))])),
        "vault_locked"
    );
}

#[test]
fn failed_save_in_add_entries_with_ids_keeps_the_old_entries() {
    let dir = TempDir::new().unwrap();
    let (mut vault, sub, _) = vault_with_broken_target(&dir);
    let keep = vault.add(password_entry("keep", "pw")).unwrap();
    break_target(&sub);
    assert_eq!(
        code(vault.add_entries_with_ids(vec![
            (uuid::Uuid::from_u128(5), password_entry("n1", "p1")),
            (uuid::Uuid::from_u128(6), password_entry("n2", "p2")),
        ])),
        "save_failed"
    );
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, keep);
}

#[test]
fn remove_entries_deletes_all_given_ids_in_one_save() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let a = vault.add(password_entry("a", "1")).unwrap();
    let b = vault.add(password_entry("b", "2")).unwrap();
    let c = vault.add(password_entry("c", "3")).unwrap();
    let before = fs::read(&path).unwrap();
    vault.remove_entries(&[a, c]).unwrap();
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, b);
    let bak = fs::read(path.with_file_name(format!("{FILE_NAME}.bak"))).unwrap();
    assert_eq!(bak, before, "exactly one save");

    // an unknown id refuses the whole call
    let before = fs::read(&path).unwrap();
    assert_eq!(
        code(vault.remove_entries(&[b, uuid::Uuid::from_u128(99)])),
        "not_found"
    );
    assert_eq!(vault.list().unwrap().len(), 1);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn a_save_that_would_exceed_the_load_cap_is_refused_and_writes_nothing() {
    let dir = TempDir::new().unwrap();
    let (mut vault, path) = new_vault(&dir);
    let keep = vault.add(password_entry("keep", "pw-one")).unwrap();
    let before = fs::read(&path).unwrap();
    let bak = path.with_file_name(format!("{FILE_NAME}.bak"));
    let bak_before = fs::read(&bak).unwrap();

    let mut huge = password_entry("huge", "pw-two");
    huge.notes = "n".repeat(usize::try_from(crate::MAX_FILE_LEN).unwrap() + 1);
    let err = vault.add(huge).unwrap_err();
    assert_eq!(err.code(), "save_failed");
    assert!(err.to_string().contains("64 MiB"), "{err}");

    assert_eq!(fs::read(&path).unwrap(), before, "vault file untouched");
    assert_eq!(fs::read(&bak).unwrap(), bak_before, ".bak untouched");
    let list = vault.list().unwrap();
    assert_eq!(list.len(), 1, "memory unchanged");
    assert_eq!(list[0].id, keep);

    // the vault stays usable
    vault.add(password_entry("after", "pw-three")).unwrap();
    let mut reopened = Vault::open(&path).unwrap().unwrap();
    reopened.unlock(&secret(PW)).unwrap();
    assert_eq!(reopened.list().unwrap().len(), 2);
}

#[test]
fn a_file_exactly_at_the_load_cap_is_allowed_one_byte_more_is_not() {
    assert!(crate::vault::check_file_len(crate::MAX_FILE_LEN, crate::MAX_FILE_LEN).is_ok());
    let err =
        crate::vault::check_file_len(crate::MAX_FILE_LEN + 1, crate::MAX_FILE_LEN).unwrap_err();
    assert_eq!(err.code(), "save_failed");
}
