//! Golden test: a v1 vault file written by the build before the generation counter existed
//! (`fixtures/v1-no-generation.fernjo`, KDF `TEST`, three entries). The owner's real vault is
//! such a file, so every later build must open it, leave its bytes alone until the first
//! save, and keep it as `.bak` when that save happens.

use std::fs;
use std::path::{Path, PathBuf};

use fernjo_vault::{EntryUpdate, FILE_NAME, Secret, Vault};
use secrecy::{ExposeSecret, SecretString};
use tempfile::TempDir;

const PASSWORD: &str = "golden fixture password";
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/v1-no-generation.fernjo"
);

fn secret(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

fn copy_fixture(dir: &TempDir) -> (PathBuf, Vec<u8>) {
    let path = dir.path().join(FILE_NAME);
    let bytes = fs::read(FIXTURE).unwrap();
    fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn bak(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".bak");
    PathBuf::from(name)
}

#[test]
fn the_v1_fixture_opens_and_lists_its_entries() {
    let dir = TempDir::new().unwrap();
    let (path, _) = copy_fixture(&dir);
    let mut vault = Vault::open(&path).unwrap().unwrap();
    vault.unlock(&secret(PASSWORD)).unwrap();
    let list = vault.list().unwrap();
    let names: Vec<&str> = list.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["Web server", "Deploy key", "Jump host"]);
    assert_eq!(list[0].username.as_deref(), Some("root"));
    assert_eq!(list[1].notes, "for CI");
    match vault.reveal(list[0].id).unwrap() {
        Secret::Password { password } => assert_eq!(password.expose_secret(), "hunter2-golden"),
        other => panic!("unexpected secret {other:?}"),
    }
    match vault.reveal(list[1].id).unwrap() {
        Secret::PrivateKey { passphrase, .. } => assert_eq!(
            passphrase.as_ref().map(ExposeSecret::expose_secret),
            Some("golden-passphrase")
        ),
        other => panic!("unexpected secret {other:?}"),
    }
}

#[test]
fn unlocking_and_locking_leave_the_file_bytes_unchanged() {
    let dir = TempDir::new().unwrap();
    let (path, original) = copy_fixture(&dir);
    let mut vault = Vault::open(&path).unwrap().unwrap();
    vault.unlock(&secret(PASSWORD)).unwrap();
    vault.lock();
    vault.unlock(&secret(PASSWORD)).unwrap();
    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!bak(&path).exists());
}

#[test]
fn the_first_save_keeps_the_fixture_as_bak_and_the_result_still_opens() {
    let dir = TempDir::new().unwrap();
    let (path, original) = copy_fixture(&dir);
    let mut vault = Vault::open(&path).unwrap().unwrap();
    vault.unlock(&secret(PASSWORD)).unwrap();
    let id = vault.list().unwrap()[2].id;
    vault
        .update(
            id,
            EntryUpdate {
                notes: Some("changed".to_owned()),
                ..EntryUpdate::default()
            },
        )
        .unwrap();
    assert_eq!(fs::read(bak(&path)).unwrap(), original);
    assert_ne!(fs::read(&path).unwrap(), original);

    let mut again = Vault::open(&path).unwrap().unwrap();
    again.unlock(&secret(PASSWORD)).unwrap();
    assert_eq!(again.list().unwrap()[2].notes, "changed");
}
