//! Head and history on disk.
//!
//! Layout: `<dir>/head.env` is the newest envelope, `<dir>/history/<seq>.env` the replaced ones
//! (only the newest [`HISTORY_KEEP`] stay). Every write goes to a random `*.tmp` name opened with
//! `create_new`, is fsynced and renamed; one lock covers the compare-and-swap.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use uuid::Uuid;

pub type GroupId = Uuid;

/// Largest envelope the hub accepts (the same number as `fernjo-sync`; pinned against it there).
pub const MAX_ENVELOPE_LEN: usize = 64 * 1024 * 1024;
pub const HISTORY_KEEP: usize = 10;

const MAGIC: &[u8; 8] = b"FERNJOSY";
const FORMAT_VERSION: u16 = 1;
const HEADER_PEEK_LEN: usize = 34;
const HEAD_FILE: &str = "head.env";
const HISTORY_DIR: &str = "history";

/// Reads group id and `seq` from the unauthenticated header; `None` when it is not an envelope
/// of format version 1.
#[must_use]
pub fn peek_header(bytes: &[u8]) -> Option<(GroupId, u64)> {
    if bytes.len() < HEADER_PEEK_LEN || &bytes[..8] != MAGIC {
        return None;
    }
    if u16::from_be_bytes([bytes[8], bytes[9]]) != FORMAT_VERSION {
        return None;
    }
    let group = Uuid::from_slice(&bytes[10..26]).ok()?;
    let seq = u64::from_be_bytes(bytes[26..34].try_into().ok()?);
    Some((group, seq))
}

/// What the writer believes the hub holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    Empty,
    Seq(u64),
}

#[derive(Debug)]
pub enum PutError {
    /// The head is not what the writer expected; nothing was written.
    Stale {
        current: Option<u64>,
    },
    SeqNotHigher,
    WrongGroup,
    TooLarge,
    Io(io::Error),
}

impl From<io::Error> for PutError {
    fn from(e: io::Error) -> Self {
        PutError::Io(e)
    }
}

pub struct HubStore {
    dir: PathBuf,
    lock: Mutex<()>,
}

impl HubStore {
    pub fn open(dir: &Path) -> io::Result<HubStore> {
        create_private_dir_all(&dir.join(HISTORY_DIR))?;
        // leftovers of a crashed write
        for d in [dir.to_path_buf(), dir.join(HISTORY_DIR)] {
            for entry in fs::read_dir(&d)? {
                let entry = entry?;
                if entry.file_name().to_string_lossy().ends_with(".tmp") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(HubStore {
            dir: dir.to_path_buf(),
            lock: Mutex::new(()),
        })
    }

    fn head_path(&self) -> PathBuf {
        self.dir.join(HEAD_FILE)
    }

    fn history_dir(&self) -> PathBuf {
        self.dir.join(HISTORY_DIR)
    }

    fn read_head_unlocked(&self) -> io::Result<Option<(u64, Vec<u8>)>> {
        match fs::read(self.head_path()) {
            Ok(bytes) => {
                let (_, seq) = peek_header(&bytes).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "head file is not an envelope")
                })?;
                Ok(Some((seq, bytes)))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn head(&self) -> io::Result<Option<(u64, Vec<u8>)>> {
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.read_head_unlocked()
    }

    /// Compare-and-swap the head. `seq` and `group` are the header values of `bytes`.
    pub fn put(
        &self,
        expected: Expected,
        seq: u64,
        group: GroupId,
        bytes: &[u8],
    ) -> Result<(), PutError> {
        if bytes.len() > MAX_ENVELOPE_LEN {
            return Err(PutError::TooLarge);
        }
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let current = self.read_head_unlocked()?;
        let current_seq = current.as_ref().map(|(s, _)| *s);
        let matches = match (expected, current_seq) {
            (Expected::Empty, None) => true,
            (Expected::Seq(e), Some(c)) => e == c,
            _ => false,
        };
        if !matches {
            return Err(PutError::Stale {
                current: current_seq,
            });
        }
        if let Some((cur_seq, old)) = &current {
            let (cur_group, _) = peek_header(old).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "head file is not an envelope")
            })?;
            if cur_group != group {
                return Err(PutError::WrongGroup);
            }
            if seq <= *cur_seq {
                return Err(PutError::SeqNotHigher);
            }
        }

        let head = self.head_path();
        let (tmp, mut file) = create_temp_file(&head)?;
        let result = (|| -> io::Result<()> {
            file.write_all(bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            if let Some((cur_seq, old)) = &current {
                write_atomic(&self.history_dir().join(format!("{cur_seq}.env")), old)?;
            }
            fs::rename(&tmp, &head)
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&tmp);
            return Err(PutError::Io(e));
        }
        let _ = sync_dir(&self.dir);
        // pruning is best effort: a leftover old file is harmless
        let _ = self.prune();
        Ok(())
    }

    fn prune(&self) -> io::Result<()> {
        let mut seqs = self.history()?;
        while seqs.len() > HISTORY_KEEP {
            let oldest = seqs.remove(0);
            fs::remove_file(self.history_dir().join(format!("{oldest}.env")))?;
        }
        Ok(())
    }

    /// The `seq` of every kept older envelope, ascending.
    pub fn history(&self) -> io::Result<Vec<u64>> {
        let mut seqs = Vec::new();
        for entry in fs::read_dir(self.history_dir())? {
            let name = entry?.file_name();
            let name = name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".env")
                && let Ok(seq) = stem.parse::<u64>()
            {
                seqs.push(seq);
            }
        }
        seqs.sort_unstable();
        Ok(seqs)
    }
}

fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// A fresh `<file>.<hex>.tmp` opened with `create_new` (mode 0600 on Unix).
pub(crate) fn create_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    for _ in 0..8 {
        let mut rnd = [0u8; 8];
        getrandom::fill(&mut rnd).map_err(|e| io::Error::other(e.to_string()))?;
        let hex: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = path.with_file_name(format!("{name}.{hex}.tmp"));
        let mut opts = OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&tmp) {
            Ok(f) => return Ok((tmp, f)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temp name",
    ))
}

/// Temp file, fsync, rename, directory fsync (Unix, best effort).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let (tmp, mut file) = create_temp_file(path)?;
    let result = (|| -> io::Result<()> {
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Some(dir) = path.parent() {
        let _ = sync_dir(dir);
    }
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::*;

    fn group() -> GroupId {
        Uuid::from_u128(7)
    }

    fn env(group: GroupId, seq: u64, extra: usize) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(MAGIC);
        v.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
        v.extend_from_slice(group.as_bytes());
        v.extend_from_slice(&seq.to_be_bytes());
        v.extend(std::iter::repeat_n(0xAB, 24 + extra));
        v
    }

    fn put(s: &HubStore, e: Expected, seq: u64) -> Result<(), PutError> {
        s.put(e, seq, group(), &env(group(), seq, 4))
    }

    fn open() -> (TempDir, HubStore) {
        let d = TempDir::new().unwrap();
        let s = HubStore::open(d.path()).unwrap();
        (d, s)
    }

    #[test]
    fn first_write_needs_empty() {
        let (_d, s) = open();
        assert!(matches!(
            put(&s, Expected::Seq(1), 2),
            Err(PutError::Stale { current: None })
        ));
        put(&s, Expected::Empty, 1).unwrap();
        assert!(matches!(
            put(&s, Expected::Empty, 2),
            Err(PutError::Stale { current: Some(1) })
        ));
        assert_eq!(s.head().unwrap().unwrap().0, 1);
    }

    #[test]
    fn second_write_with_the_current_seq_succeeds() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 1).unwrap();
        put(&s, Expected::Seq(1), 2).unwrap();
        let (seq, bytes) = s.head().unwrap().unwrap();
        assert_eq!(seq, 2);
        assert_eq!(bytes, env(group(), 2, 4));
        assert_eq!(s.history().unwrap(), vec![1]);
    }

    #[test]
    fn a_stale_expected_seq_is_refused_and_nothing_changes() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 1).unwrap();
        put(&s, Expected::Seq(1), 2).unwrap();
        let before = s.head().unwrap();
        assert!(matches!(
            put(&s, Expected::Seq(1), 3),
            Err(PutError::Stale { current: Some(2) })
        ));
        assert_eq!(s.head().unwrap(), before);
        assert_eq!(s.history().unwrap(), vec![1]);
    }

    #[test]
    fn a_seq_not_higher_is_refused() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 5).unwrap();
        assert!(matches!(
            put(&s, Expected::Seq(5), 5),
            Err(PutError::SeqNotHigher)
        ));
        assert!(matches!(
            put(&s, Expected::Seq(5), 4),
            Err(PutError::SeqNotHigher)
        ));
        assert_eq!(s.head().unwrap().unwrap().0, 5);
        assert!(s.history().unwrap().is_empty());
    }

    #[test]
    fn the_first_write_fixes_the_group() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 1).unwrap();
        let other = Uuid::from_u128(8);
        assert!(matches!(
            s.put(Expected::Seq(1), 2, other, &env(other, 2, 0)),
            Err(PutError::WrongGroup)
        ));
        assert_eq!(s.head().unwrap().unwrap().0, 1);
    }

    #[test]
    fn only_the_newest_10_envelopes_are_kept() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 1).unwrap();
        for seq in 2..=15 {
            put(&s, Expected::Seq(seq - 1), seq).unwrap();
        }
        assert_eq!(s.history().unwrap(), (5..=14).collect::<Vec<u64>>());
        assert_eq!(s.head().unwrap().unwrap().0, 15);
    }

    #[test]
    fn a_reopened_store_keeps_head_and_history() {
        let d = TempDir::new().unwrap();
        {
            let s = HubStore::open(d.path()).unwrap();
            put(&s, Expected::Empty, 1).unwrap();
            put(&s, Expected::Seq(1), 2).unwrap();
        }
        let s = HubStore::open(d.path()).unwrap();
        assert_eq!(s.head().unwrap().unwrap().0, 2);
        assert_eq!(s.history().unwrap(), vec![1]);
        put(&s, Expected::Seq(2), 3).unwrap();
        let names: Vec<_> = fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
    }

    #[test]
    fn an_oversize_body_is_too_large() {
        let (_d, s) = open();
        let big = vec![0u8; MAX_ENVELOPE_LEN + 1];
        assert!(matches!(
            s.put(Expected::Empty, 1, group(), &big),
            Err(PutError::TooLarge)
        ));
        assert!(s.head().unwrap().is_none());
    }

    #[test]
    fn two_threads_writing_the_same_expected_seq_one_wins() {
        let (_d, s) = open();
        put(&s, Expected::Empty, 1).unwrap();
        let s = Arc::new(s);
        let handles: Vec<_> = [2u64, 3]
            .into_iter()
            .map(|seq| {
                let s = Arc::clone(&s);
                std::thread::spawn(move || put(&s, Expected::Seq(1), seq))
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1);
        assert!(
            results
                .iter()
                .any(|r| matches!(r, Err(PutError::Stale { .. })))
        );
        assert_eq!(s.history().unwrap(), vec![1]);
    }
}
