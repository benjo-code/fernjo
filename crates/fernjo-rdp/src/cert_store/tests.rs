use std::fs;
use std::path::Path;

use super::*;

const FP_A: &str = "sha256:aa11";
const FP_B: &str = "sha256:bb22";

fn setup() -> (tempfile::TempDir, CertStore, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rdp_certs.confirmed");
    (dir, CertStore::open(path.clone()), path)
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

#[test]
fn unknown_then_pinned_then_match() {
    let (_d, store, path) = setup();
    assert_eq!(store.check("dc01", 3389, FP_A).unwrap(), CertCheck::Unknown);
    store.pin("dc01", 3389, FP_A).unwrap();
    assert_eq!(store.check("dc01", 3389, FP_A).unwrap(), CertCheck::Match);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "dc01:3389 sha256:aa11\n"
    );
    // the host is not case sensitive, the port is part of the identity
    assert_eq!(store.check("DC01", 3389, FP_A).unwrap(), CertCheck::Match);
    assert_eq!(store.check("dc01", 3390, FP_A).unwrap(), CertCheck::Unknown);
    assert_eq!(store.check("dc02", 3389, FP_A).unwrap(), CertCheck::Unknown);
}

#[test]
fn a_different_fingerprint_is_changed_with_the_old_one() {
    let (_d, store, _p) = setup();
    store.pin("dc01", 3389, FP_A).unwrap();
    assert_eq!(
        store.check("dc01", 3389, FP_B).unwrap(),
        CertCheck::Changed {
            old: FP_A.to_string()
        }
    );
}

#[test]
fn replace_overwrites_the_pin() {
    let (_d, store, path) = setup();
    store.pin("dc01", 3389, FP_A).unwrap();
    store.pin("other", 3389, FP_A).unwrap();
    store.replace("dc01", 3389, FP_B).unwrap();
    assert_eq!(store.check("dc01", 3389, FP_B).unwrap(), CertCheck::Match);
    assert_eq!(
        store.check("dc01", 3389, FP_A).unwrap(),
        CertCheck::Changed {
            old: FP_B.to_string()
        }
    );
    // one line per pin: the old line is gone, the other host is untouched
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.contains("other:3389 sha256:aa11"));
    // pinning an already pinned host again does not duplicate it either
    store.pin("dc01", 3389, FP_B).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 2);
}

#[test]
fn a_file_larger_than_the_cap_is_unreadable_and_untouched() {
    let (dir, store, path) = setup();
    let file = fs::File::create(&path).unwrap();
    file.set_len(MAX_FILE_LEN + 1).unwrap();
    drop(file);
    assert_eq!(store.check("dc01", 3389, FP_A).unwrap_err().code(), "io");
    assert_eq!(store.pin("dc01", 3389, FP_A).unwrap_err().code(), "io");
    assert_eq!(fs::metadata(&path).unwrap().len(), MAX_FILE_LEN + 1);
    assert_eq!(names(dir.path()), ["rdp_certs.confirmed"]);
}

#[test]
fn a_host_with_a_space_is_invalid_host() {
    let (_d, store, path) = setup();
    for bad in [
        "a b",
        "a\tb",
        "a\nb",
        "a\rb",
        "",
        "a\u{7}b",
        &"h".repeat(256),
    ] {
        assert_eq!(
            store.check(bad, 3389, FP_A).unwrap_err().code(),
            "invalid_host",
            "{bad:?}"
        );
        assert_eq!(
            store.pin(bad, 3389, FP_A).unwrap_err().code(),
            "invalid_host",
            "{bad:?}"
        );
    }
    assert!(!path.exists());
}

#[test]
fn a_fingerprint_that_could_start_a_second_line_is_refused() {
    let (_d, store, path) = setup();
    assert!(
        store
            .pin("dc01", 3389, "sha256:aa\nevil:1 sha256:bb")
            .is_err()
    );
    assert!(store.pin("dc01", 3389, "").is_err());
    assert!(!path.exists());
}

#[test]
fn the_pin_survives_reopening_the_file() {
    let (_d, store, path) = setup();
    store.pin("dc01", 3389, FP_A).unwrap();
    drop(store);
    let again = CertStore::open(path);
    assert_eq!(again.check("dc01", 3389, FP_A).unwrap(), CertCheck::Match);
}

#[test]
fn in_memory_keeps_pins_for_the_run_only() {
    let store = CertStore::in_memory();
    store.pin("dc01", 3389, FP_A).unwrap();
    assert_eq!(store.check("dc01", 3389, FP_A).unwrap(), CertCheck::Match);
    // clones share the pins (the manager hands a clone to every connect task)
    assert_eq!(
        store.clone().check("dc01", 3389, FP_A).unwrap(),
        CertCheck::Match
    );
    // a new in-memory store starts empty
    assert_eq!(
        CertStore::in_memory().check("dc01", 3389, FP_A).unwrap(),
        CertCheck::Unknown
    );
}

#[test]
fn other_lines_comments_and_the_backup_are_kept() {
    let (dir, store, path) = setup();
    fs::write(&path, "# mine\nold:3389 sha256:cc33\n\nnot a pin\n").unwrap();
    store.pin("dc01", 3389, FP_A).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("# mine\nold:3389 sha256:cc33\n\nnot a pin\n"),
        "{text}"
    );
    assert!(text.ends_with("dc01:3389 sha256:aa11\n"), "{text}");
    // the previous bytes are the backup, and no temp file is left behind
    assert_eq!(
        fs::read_to_string(dir.path().join("rdp_certs.confirmed.bak")).unwrap(),
        "# mine\nold:3389 sha256:cc33\n\nnot a pin\n"
    );
    assert_eq!(
        names(dir.path()),
        ["rdp_certs.confirmed", "rdp_certs.confirmed.bak"]
    );
}

#[test]
fn a_missing_folder_is_created_on_the_first_pin() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir
        .path()
        .join("fresh")
        .join("device")
        .join("rdp_certs.confirmed");
    let store = CertStore::open(path.clone());
    assert_eq!(store.check("dc01", 3389, FP_A).unwrap(), CertCheck::Unknown);
    store.pin("dc01", 3389, FP_A).unwrap();
    assert!(path.exists());
}

#[cfg(unix)]
#[test]
fn the_file_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let (_d, store, path) = setup();
    store.pin("dc01", 3389, FP_A).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
