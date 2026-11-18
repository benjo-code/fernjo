//! Device-local marks of the highest vault generation this device has seen.
//!
//! A vault file carries a generation counter inside its encrypted body. A sync tool that puts
//! an older copy of the file back would otherwise go unnoticed; the mark kept here (outside the
//! synced data folder) is what an older file is compared against. The marks are an integrity
//! aid, never a requirement: a store that cannot be read or written means less protection, not
//! a failed unlock or save. They hold no secret.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::format::read_capped;
use crate::vault::{VaultId, write_atomic};

/// File name of [`FileGenerationStore`] inside the device-local folder.
pub const MARKS_FILE_NAME: &str = "vault-marks.json";

/// The marks file is tiny (one number per vault ever opened); anything larger is not ours.
const MAX_MARKS_LEN: u64 = 1024 * 1024;

const SCHEMA_VERSION: u32 = 1;

/// The highest generation seen per vault id.
pub trait GenerationStore: Send + Sync {
    /// The mark, or `None` when it is unknown or the store is unavailable.
    fn get(&self, vault: VaultId) -> Option<u64>;

    /// Raises the mark to `generation` (never lowers it). Best effort: failures are ignored.
    fn raise(&self, vault: VaultId, generation: u64);
}

/// Marks in memory only (tests, and the fallback when there is no device-local folder).
#[derive(Debug, Default)]
pub struct MemoryGenerationStore {
    marks: Mutex<HashMap<VaultId, u64>>,
}

impl GenerationStore for MemoryGenerationStore {
    fn get(&self, vault: VaultId) -> Option<u64> {
        lock(&self.marks).get(&vault).copied()
    }

    fn raise(&self, vault: VaultId, generation: u64) {
        let mut marks = lock(&self.marks);
        let slot = marks.entry(vault).or_insert(0);
        *slot = (*slot).max(generation);
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MarksFile {
    schema_version: u32,
    marks: BTreeMap<String, u64>,
}

/// Marks in `<device folder>/vault-marks.json` (`{"schemaVersion":1,"marks":{"<vault id>":N}}`).
///
/// The folder is created on the first write (private modes on Unix). The file is re-read right
/// before every write so two processes only ever raise it, and an unreadable, corrupt or
/// newer-schema file counts as empty. The highest value seen or written by this process is also
/// kept in memory, so a folder that cannot be written still protects against a rollback during
/// the run.
#[derive(Debug)]
pub struct FileGenerationStore {
    path: PathBuf,
    seen: Mutex<HashMap<VaultId, u64>>,
}

impl FileGenerationStore {
    /// A store at exactly `path`.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// A store in `device_dir` under [`MARKS_FILE_NAME`].
    #[must_use]
    pub fn in_dir(device_dir: &Path) -> Self {
        Self::new(device_dir.join(MARKS_FILE_NAME))
    }

    fn read(&self) -> MarksFile {
        read_capped(&self.path, MAX_MARKS_LEN)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<MarksFile>(&bytes).ok())
            .filter(|f| f.schema_version == SCHEMA_VERSION)
            .unwrap_or_else(|| MarksFile {
                schema_version: SCHEMA_VERSION,
                marks: BTreeMap::new(),
            })
    }
}

impl GenerationStore for FileGenerationStore {
    fn get(&self, vault: VaultId) -> Option<u64> {
        let on_disk = self.read().marks.get(&vault.to_string()).copied();
        let in_memory = lock(&self.seen).get(&vault).copied();
        on_disk.max(in_memory)
    }

    fn raise(&self, vault: VaultId, generation: u64) {
        let mut seen = lock(&self.seen);
        let slot = seen.entry(vault).or_insert(0);
        *slot = (*slot).max(generation);
        let mut file = self.read();
        let current = file.marks.entry(vault.to_string()).or_insert(0);
        if *current >= generation {
            return;
        }
        *current = generation;
        if let Ok(bytes) = serde_json::to_vec(&file) {
            // best effort: the in-memory mark above still protects this run
            let _ = write_atomic(&self.path, &bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn memory_store_only_ever_raises_and_is_per_vault() {
        let store = MemoryGenerationStore::default();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(store.get(a), None);
        store.raise(a, 5);
        store.raise(a, 3);
        store.raise(b, 1);
        assert_eq!(store.get(a), Some(5));
        assert_eq!(store.get(b), Some(1));
    }

    #[test]
    fn file_store_round_trips_and_is_written_in_the_documented_shape() {
        let dir = TempDir::new().unwrap();
        let device = dir.path().join("device");
        let store = FileGenerationStore::in_dir(&device);
        let id = Uuid::new_v4();
        assert_eq!(store.get(id), None);
        store.raise(id, 4);
        store.raise(id, 2);
        assert_eq!(store.get(id), Some(4));
        let text = fs::read_to_string(device.join(MARKS_FILE_NAME)).unwrap();
        assert_eq!(
            text,
            format!(r#"{{"schemaVersion":1,"marks":{{"{id}":4}}}}"#)
        );
        // a second handle (another process) sees it
        assert_eq!(FileGenerationStore::in_dir(&device).get(id), Some(4));
    }

    #[test]
    fn a_corrupt_or_newer_file_counts_as_empty_and_is_replaced_by_the_next_raise() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(MARKS_FILE_NAME);
        let id = Uuid::new_v4();
        for content in [
            &b"not json"[..],
            &b""[..],
            &br#"{"schemaVersion":2,"marks":{}}"#[..],
        ] {
            fs::write(&path, content).unwrap();
            let store = FileGenerationStore::new(path.clone());
            assert_eq!(store.get(id), None);
            store.raise(id, 3);
            assert_eq!(FileGenerationStore::new(path.clone()).get(id), Some(3));
        }
    }

    #[test]
    fn raise_rereads_the_file_so_another_writers_marks_survive() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(MARKS_FILE_NAME);
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let first = FileGenerationStore::new(path.clone());
        let second = FileGenerationStore::new(path.clone());
        first.raise(a, 7);
        second.raise(b, 2); // `second` never saw `a`
        assert_eq!(first.get(a), Some(7));
        assert_eq!(first.get(b), Some(2));
        second.raise(a, 5); // lower than what is on disk
        assert_eq!(FileGenerationStore::new(path).get(a), Some(7));
    }

    #[test]
    fn an_unwritable_location_keeps_the_mark_in_memory() {
        let dir = TempDir::new().unwrap();
        // the "folder" is a file, so nothing can be created below it
        let blocker = dir.path().join("blocker");
        fs::write(&blocker, b"x").unwrap();
        let store = FileGenerationStore::in_dir(&blocker);
        let id = Uuid::new_v4();
        store.raise(id, 9);
        assert_eq!(store.get(id), Some(9));
        assert_eq!(FileGenerationStore::in_dir(&blocker).get(id), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_device_folder_and_file_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let device = dir.path().join("device");
        FileGenerationStore::in_dir(&device).raise(Uuid::new_v4(), 1);
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&device), 0o700);
        assert_eq!(mode(&device.join(MARKS_FILE_NAME)), 0o600);
    }
}
