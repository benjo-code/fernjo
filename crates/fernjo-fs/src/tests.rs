use std::fs;
use std::io;
use std::path::Path;

use tempfile::TempDir;

use crate::{FileMode, NOT_REGULAR, Parent, WriteOptions, read_capped, with_suffix, write_atomic};

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

const NO_BAK: WriteOptions = WriteOptions {
    parent: Parent::CreatePrivate,
    backup: false,
    file_mode: FileMode::OwnerOnly,
};

#[test]
fn read_capped_accepts_up_to_the_cap_and_refuses_one_byte_more() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("f");
    fs::write(&path, [7u8; 10]).unwrap();
    assert_eq!(read_capped(&path, 10).unwrap(), vec![7u8; 10]);
    let err = read_capped(&path, 9).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "the file is too large");
    fs::write(&path, []).unwrap();
    assert!(read_capped(&path, 0).unwrap().is_empty());
    let missing = read_capped(&dir.path().join("none"), 10).unwrap_err();
    assert_eq!(missing.kind(), io::ErrorKind::NotFound);
}

#[test]
fn write_uses_a_fresh_temp_name_and_leaves_none_behind() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("file.bin");
    // a leftover fixed-name temp file from an older writer is never reused or touched
    fs::write(with_suffix(&path, ".tmp"), b"old leftover").unwrap();
    write_atomic(&path, b"one", NO_BAK).unwrap();
    write_atomic(&path, b"two", NO_BAK).unwrap();
    // the bytes exactly, no newline added
    assert_eq!(fs::read(&path).unwrap(), b"two");
    assert_eq!(names(dir.path()), ["file.bin", "file.bin.tmp"]);
    assert_eq!(
        fs::read(with_suffix(&path, ".tmp")).unwrap(),
        b"old leftover"
    );
}

#[test]
fn write_keeps_the_previous_content_as_bak_only_when_asked() {
    let dir = TempDir::new().unwrap();
    let with_bak = dir.path().join("a.json");
    write_atomic(&with_bak, b"first", WriteOptions::data_file()).unwrap();
    assert!(
        !with_suffix(&with_bak, ".bak").exists(),
        "nothing to keep yet"
    );
    write_atomic(&with_bak, b"second", WriteOptions::data_file()).unwrap();
    assert_eq!(fs::read(&with_bak).unwrap(), b"second");
    assert_eq!(fs::read(with_suffix(&with_bak, ".bak")).unwrap(), b"first");
    write_atomic(&with_bak, b"third", WriteOptions::data_file()).unwrap();
    assert_eq!(fs::read(with_suffix(&with_bak, ".bak")).unwrap(), b"second");

    let without = dir.path().join("b.json");
    write_atomic(&without, b"first", NO_BAK).unwrap();
    write_atomic(&without, b"second", NO_BAK).unwrap();
    assert_eq!(fs::read(&without).unwrap(), b"second");
    assert_eq!(names(dir.path()), ["a.json", "a.json.bak", "b.json"]);
}

#[test]
fn a_failed_write_leaves_the_target_and_no_temp_file() {
    // the `.bak` step fails: a folder sits at the `.bak` name
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x");
    write_atomic(&path, b"old", WriteOptions::data_file()).unwrap();
    fs::create_dir(with_suffix(&path, ".bak")).unwrap();
    assert!(write_atomic(&path, b"new", WriteOptions::data_file()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"old");
    assert_eq!(names(dir.path()), ["x", "x.bak"]);

    // the final rename fails: a folder sits at the target name
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("y");
    fs::create_dir(&path).unwrap();
    assert!(write_atomic(&path, b"new", NO_BAK).is_err());
    assert!(path.is_dir());
    assert_eq!(names(dir.path()), ["y"]);
}

#[test]
fn must_exist_parent_is_not_created() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing").join("f");
    let options = WriteOptions {
        parent: Parent::MustExist,
        backup: false,
        file_mode: FileMode::OwnerOnly,
    };
    let err = write_atomic(&path, b"x", options).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
    assert!(!dir.path().join("missing").exists());
    assert!(names(dir.path()).is_empty());

    // an existing parent works
    let path = dir.path().join("f");
    write_atomic(&path, b"x", options).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"x");
}

#[test]
fn create_default_parent_creates_missing_folders() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("f");
    let options = WriteOptions {
        parent: Parent::CreateDefault,
        backup: false,
        file_mode: FileMode::Default,
    };
    write_atomic(&path, b"x", options).unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"x");

    let private = dir.path().join("c").join("d").join("f");
    write_atomic(&private, b"y", WriteOptions::data_file()).unwrap();
    assert_eq!(fs::read(&private).unwrap(), b"y");
}

#[test]
fn read_capped_refuses_what_is_not_a_regular_file() {
    // a folder stands in for every non-regular entry here (Windows needs admin rights for a
    // symlink); the symlink and FIFO cases are the Unix tests below
    let dir = TempDir::new().unwrap();
    let folder = dir.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let err = read_capped(&folder, 1024).unwrap_err();
    assert_eq!(err.to_string(), NOT_REGULAR);
    assert_ne!(
        err.kind(),
        io::ErrorKind::InvalidData,
        "InvalidData is reserved for oversize"
    );
    assert_ne!(err.kind(), io::ErrorKind::NotFound);
}

#[test]
fn write_refuses_a_target_that_is_not_a_regular_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("x");
    fs::create_dir(&path).unwrap();
    for options in [WriteOptions::data_file(), NO_BAK] {
        let err = write_atomic(&path, b"new", options).unwrap_err();
        assert_eq!(err.to_string(), NOT_REGULAR);
        assert!(path.is_dir());
        assert_eq!(names(dir.path()), ["x"], "no temp file and no .bak left");
    }
}

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::{NO_BAK, NOT_REGULAR, names};
    use crate::{
        FileMode, Parent, WriteOptions, create_temp_file_from, read_capped, with_suffix,
        write_atomic,
    };

    /// A FIFO at `path` (`mkfifo` is part of every Linux and macOS base system).
    fn mkfifo(path: &Path) {
        let status = Command::new("mkfifo").arg(path).status().unwrap();
        assert!(status.success(), "mkfifo failed");
    }

    /// Runs `f` on its own thread and fails the test when it does not finish within 5 s (a FIFO
    /// that is opened for reading blocks until a writer appears, so a hang means it was opened).
    fn finishes_quickly<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the call blocked on the FIFO")
    }

    #[test]
    fn a_symlink_at_the_target_is_not_copied_into_bak() {
        let dir = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("id_ed25519");
        fs::write(&secret, b"SECRET KEY BYTES").unwrap();
        let path = dir.path().join("known_hosts");
        symlink(&secret, &path).unwrap();

        for options in [WriteOptions::data_file(), NO_BAK] {
            let err = write_atomic(&path, b"new", options).unwrap_err();
            assert_eq!(err.to_string(), NOT_REGULAR);
        }

        assert_eq!(fs::read(&secret).unwrap(), b"SECRET KEY BYTES");
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link is refused, not replaced"
        );
        assert_eq!(names(dir.path()), ["known_hosts"], "no .bak, no temp file");
    }

    #[test]
    fn read_capped_does_not_follow_a_symlink_at_the_target() {
        let dir = TempDir::new().unwrap();
        let secret = dir.path().join("secret");
        fs::write(&secret, b"SECRET").unwrap();
        let link = dir.path().join("link");
        symlink(&secret, &link).unwrap();
        assert_eq!(
            read_capped(&link, 1024).unwrap_err().to_string(),
            NOT_REGULAR
        );

        // a dangling link is refused too, it never reads as a missing file
        let dangling = dir.path().join("dangling");
        symlink(dir.path().join("nothing"), &dangling).unwrap();
        let err = read_capped(&dangling, 1024).unwrap_err();
        assert_ne!(err.kind(), std::io::ErrorKind::NotFound);

        // a link to a device is refused before any byte is read
        let zero = dir.path().join("zero");
        symlink("/dev/zero", &zero).unwrap();
        assert_eq!(
            read_capped(&zero, 1024).unwrap_err().to_string(),
            NOT_REGULAR
        );
    }

    #[test]
    fn read_capped_refuses_a_fifo_without_blocking() {
        let dir = TempDir::new().unwrap();
        let fifo: PathBuf = dir.path().join("fifo");
        mkfifo(&fifo);
        let err = finishes_quickly(move || read_capped(&fifo, 1024)).unwrap_err();
        assert_eq!(err.to_string(), NOT_REGULAR);
    }

    #[test]
    fn the_writer_refuses_a_fifo_at_the_target_without_blocking() {
        let dir = TempDir::new().unwrap();
        let fifo = dir.path().join("fifo");
        mkfifo(&fifo);
        let target = fifo.clone();
        let err = finishes_quickly(move || write_atomic(&target, b"x", WriteOptions::data_file()))
            .unwrap_err();
        assert_eq!(err.to_string(), NOT_REGULAR);
        assert_eq!(names(dir.path()), ["fifo"]);
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn owner_only_files_and_created_folders_are_0600_and_0700() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a");
        let b = a.join("b");
        let path = b.join("f");
        write_atomic(&path, b"one", WriteOptions::data_file()).unwrap();
        write_atomic(&path, b"two", WriteOptions::data_file()).unwrap();
        assert_eq!(mode(&a), 0o700);
        assert_eq!(mode(&b), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&with_suffix(&path, ".bak")), 0o600);
    }

    #[test]
    fn an_existing_folder_keeps_its_mode() {
        let dir = TempDir::new().unwrap();
        let existing = dir.path().join("shared");
        fs::create_dir(&existing).unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o755)).unwrap();
        write_atomic(&existing.join("f"), b"x", WriteOptions::data_file()).unwrap();
        assert_eq!(mode(&existing), 0o755);
        let options = WriteOptions {
            parent: Parent::CreateDefault,
            backup: false,
            file_mode: FileMode::Default,
        };
        write_atomic(&existing.join("g"), b"x", options).unwrap();
        assert_eq!(mode(&existing), 0o755);
    }

    #[test]
    fn a_symlink_at_the_bak_name_is_replaced_not_followed() {
        let dir = TempDir::new().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim data").unwrap();
        let path = dir.path().join("x");
        let bak = with_suffix(&path, ".bak");
        symlink(&victim, &bak).unwrap();

        write_atomic(&path, b"one", WriteOptions::data_file()).unwrap();
        write_atomic(&path, b"two", WriteOptions::data_file()).unwrap();

        assert_eq!(fs::read(&victim).unwrap(), b"victim data");
        let meta = fs::symlink_metadata(&bak).unwrap();
        assert!(meta.file_type().is_file(), "the link was replaced");
        assert_eq!(fs::read(&bak).unwrap(), b"one");
        assert_eq!(fs::read(&path).unwrap(), b"two");
        assert_eq!(names(dir.path()), ["victim", "x", "x.bak"]);
    }

    #[test]
    fn a_symlink_at_a_temp_name_is_never_followed() {
        let dir = TempDir::new().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, b"victim data").unwrap();
        let planted = dir.path().join("x.0000000000000000.tmp");
        symlink(&victim, &planted).unwrap();
        let fresh = dir.path().join("x.1111111111111111.tmp");

        let mut candidates = vec![fresh.clone(), planted.clone()];
        let (tmp, file) =
            create_temp_file_from(|| Ok(candidates.pop().unwrap()), FileMode::OwnerOnly).unwrap();
        drop(file);

        assert_eq!(tmp, fresh, "the taken name was skipped");
        assert_eq!(fs::read(&victim).unwrap(), b"victim data");
        assert!(
            fs::symlink_metadata(&planted)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(mode(&fresh), 0o600);
    }
}
