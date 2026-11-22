//! Device-local sync state: `sync.json` (where the hub is, who this device is, the last `seq`
//! seen) and `sync-base.json` (the base snapshot of the last successful sync). Both live in the
//! device folder, never in the synced data folder, and hold no secret: the hub secret and the
//! sync key are in the OS keychain (`secrets.rs`).
//!
//! A file that cannot be read, does not parse, is too large or has a newer `schema` is
//! `state_unusable` and is never overwritten.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::envelope::GroupId;
use crate::error::SyncError;
use crate::files::{read_capped, write_atomic};
use crate::snapshot::{DeviceInfo, Snapshot};

/// File name of the pairing state in the device folder.
pub const STATE_FILE: &str = "sync.json";
/// File name of the base snapshot in the device folder.
pub const BASE_FILE: &str = "sync-base.json";
/// The schema number both files are written with and the highest this build reads.
pub const SCHEMA: u32 = 1;

/// What this device knows about its sync group. Holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncState {
    pub url: String,
    pub ca_pem: Option<String>,
    pub device: DeviceInfo,
    pub group: GroupId,
    /// The highest `seq` this device has seen or written; it never lowers.
    pub last_seq: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StateFile {
    schema: u32,
    url: String,
    #[serde(default)]
    ca_pem: Option<String>,
    device: DeviceInfo,
    group: GroupId,
    last_seq: u64,
}

#[derive(Serialize)]
struct BaseOut {
    schema: u32,
    snapshot: serde_json::Value,
}

fn unusable(reason: impl ToString) -> SyncError {
    SyncError::StateUnusable(reason.to_string())
}

fn write_failed(e: &io::Error) -> SyncError {
    SyncError::StateWriteFailed(e.to_string())
}

/// Parses `bytes` as JSON, refuses a newer `schema` and returns the value.
fn checked_value(bytes: &[u8]) -> Result<serde_json::Value, SyncError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|e| unusable(format!("not valid JSON ({e})")))?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| unusable("missing schema"))?;
    if schema > u64::from(SCHEMA) {
        return Err(unusable(format!(
            "schema {schema} is newer than the supported {SCHEMA}"
        )));
    }
    Ok(value)
}

/// The device-local sync files in one folder.
#[derive(Debug, Clone)]
pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    /// `device_dir` need not exist yet; the first save creates it.
    #[must_use]
    pub fn new(device_dir: PathBuf) -> StateStore {
        StateStore { dir: device_dir }
    }

    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, SyncError> {
        match read_capped(&self.dir.join(name)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(unusable(format!("{name} cannot be read ({e})"))),
        }
    }

    /// The saved state; `None` when this device is not paired.
    ///
    /// # Errors
    /// `state_unusable` for an unreadable, damaged or newer file.
    pub fn load(&self) -> Result<Option<SyncState>, SyncError> {
        let Some(bytes) = self.read(STATE_FILE)? else {
            return Ok(None);
        };
        let value = checked_value(&bytes)?;
        let file: StateFile = serde_json::from_value(value)
            .map_err(|e| unusable(format!("{STATE_FILE} is damaged ({e})")))?;
        Ok(Some(SyncState {
            url: file.url,
            ca_pem: file.ca_pem,
            device: file.device,
            group: file.group,
            last_seq: file.last_seq,
        }))
    }

    /// Saves the state. A stored `last_seq` of the same group that is higher than the given one
    /// is kept, so the value only ever rises.
    ///
    /// # Errors
    /// `state_unusable` when the existing file is unusable (it is left untouched),
    /// `state_write_failed` when writing fails.
    pub fn save(&self, state: &SyncState) -> Result<(), SyncError> {
        let mut last_seq = state.last_seq;
        if let Some(existing) = self.load()?
            && existing.group == state.group
        {
            last_seq = last_seq.max(existing.last_seq);
        }
        let file = StateFile {
            schema: SCHEMA,
            url: state.url.clone(),
            ca_pem: state.ca_pem.clone(),
            device: state.device.clone(),
            group: state.group,
            last_seq,
        };
        let bytes = serde_json::to_vec_pretty(&file).map_err(unusable)?;
        write_atomic(&self.dir.join(STATE_FILE), &bytes).map_err(|e| write_failed(&e))
    }

    /// The base snapshot of the last successful sync; `None` when there is none.
    ///
    /// # Errors
    /// `state_unusable` for an unreadable, damaged or newer file.
    pub fn load_base(&self) -> Result<Option<Snapshot>, SyncError> {
        let Some(bytes) = self.read(BASE_FILE)? else {
            return Ok(None);
        };
        let value = checked_value(&bytes)?;
        let snapshot = value
            .get("snapshot")
            .ok_or_else(|| unusable(format!("{BASE_FILE} has no snapshot")))?;
        let plain = serde_json::to_vec(snapshot).map_err(unusable)?;
        let snapshot = Snapshot::from_plaintext(&plain)
            .map_err(|e| unusable(format!("{BASE_FILE} is damaged ({e})")))?;
        Ok(Some(snapshot))
    }

    /// Saves the base snapshot.
    ///
    /// # Errors
    /// `state_unusable` when the existing file is unusable (it is left untouched),
    /// `state_write_failed` when writing fails.
    pub fn save_base(&self, base: &Snapshot) -> Result<(), SyncError> {
        self.load_base()?;
        let plain = base.to_plaintext()?;
        let snapshot: serde_json::Value = serde_json::from_slice(&plain).map_err(unusable)?;
        let bytes = serde_json::to_vec(&BaseOut {
            schema: SCHEMA,
            snapshot,
        })
        .map_err(unusable)?;
        write_atomic(&self.dir.join(BASE_FILE), &bytes).map_err(|e| write_failed(&e))
    }

    /// Removes both files and their temp-file leftovers; absent files are fine.
    ///
    /// # Errors
    /// `state_write_failed` when something cannot be removed.
    pub fn clear(&self) -> Result<(), SyncError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(write_failed(&e)),
        };
        for entry in entries {
            let entry = entry.map_err(|e| write_failed(&e))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let ours = [STATE_FILE, BASE_FILE].iter().any(|base| {
                name == *base || (name.starts_with(&format!("{base}.")) && name.ends_with(".tmp"))
            });
            if ours {
                match fs::remove_file(entry.path()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(write_failed(&e)),
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use fernjo_core::Tree;
    use uuid::Uuid;

    use super::*;
    use crate::snapshot::{DeviceInfo, Snapshot};

    fn state(seq: u64) -> SyncState {
        SyncState {
            url: "https://hub.example:8443".into(),
            ca_pem: Some("-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".into()),
            device: DeviceInfo {
                id: Uuid::from_u128(1),
                name: "laptop".into(),
            },
            group: Uuid::from_u128(2),
            last_seq: seq,
        }
    }

    fn base(vault: Option<Vec<u8>>) -> Snapshot {
        Snapshot {
            format_version: 1,
            group_id: Uuid::from_u128(2),
            seq: 7,
            writer: DeviceInfo {
                id: Uuid::from_u128(1),
                name: "laptop".into(),
            },
            written_at: "2026-11-22T10:00:00Z".into(),
            connections: Tree::default(),
            vault,
        }
    }

    fn store() -> (tempfile::TempDir, StateStore) {
        let dir = tempfile::tempdir().unwrap();
        // the device folder does not exist yet
        let store = StateStore::new(dir.path().join("fernjo-device"));
        (dir, store)
    }

    #[test]
    fn state_round_trips_and_last_seq_never_lowers() {
        let (_dir, store) = store();
        assert_eq!(store.load().unwrap(), None);
        store.save(&state(5)).unwrap();
        assert_eq!(store.load().unwrap(), Some(state(5)));
        store.save(&state(3)).unwrap();
        assert_eq!(store.load().unwrap().unwrap().last_seq, 5);
        store.save(&state(9)).unwrap();
        assert_eq!(store.load().unwrap().unwrap().last_seq, 9);
        // another group starts over
        let mut other = state(1);
        other.group = Uuid::from_u128(3);
        store.save(&other).unwrap();
        assert_eq!(store.load().unwrap(), Some(other));
    }

    #[test]
    fn a_corrupt_state_file_is_state_unusable_and_is_not_overwritten() {
        let (_dir, store) = store();
        store.save(&state(1)).unwrap();
        let path = store.dir.join(STATE_FILE);
        fs::write(&path, b"{ not json").unwrap();
        assert_eq!(store.load().unwrap_err().code(), "state_unusable");
        assert_eq!(store.save(&state(2)).unwrap_err().code(), "state_unusable");
        assert_eq!(fs::read(&path).unwrap(), b"{ not json");

        // the base file follows the same rule
        let base_path = store.dir.join(BASE_FILE);
        fs::write(&base_path, b"garbage").unwrap();
        assert_eq!(store.load_base().unwrap_err().code(), "state_unusable");
        assert_eq!(
            store.save_base(&base(None)).unwrap_err().code(),
            "state_unusable"
        );
        assert_eq!(fs::read(&base_path).unwrap(), b"garbage");
    }

    #[test]
    fn a_newer_schema_state_file_is_state_unusable() {
        let (_dir, store) = store();
        store.save(&state(1)).unwrap();
        store.save_base(&base(None)).unwrap();
        for name in [STATE_FILE, BASE_FILE] {
            let path = store.dir.join(name);
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            value["schema"] = 2.into();
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert_eq!(store.load().unwrap_err().code(), "state_unusable");
        assert_eq!(store.load_base().unwrap_err().code(), "state_unusable");
        let before = fs::read(store.dir.join(STATE_FILE)).unwrap();
        assert_eq!(store.save(&state(4)).unwrap_err().code(), "state_unusable");
        assert_eq!(fs::read(store.dir.join(STATE_FILE)).unwrap(), before);
    }

    #[test]
    fn base_round_trips_with_a_vault_and_without() {
        let (_dir, store) = store();
        assert_eq!(store.load_base().unwrap(), None);
        store.save_base(&base(Some(vec![1, 2, 3, 255]))).unwrap();
        assert_eq!(
            store.load_base().unwrap(),
            Some(base(Some(vec![1, 2, 3, 255])))
        );
        store.save_base(&base(None)).unwrap();
        assert_eq!(store.load_base().unwrap(), Some(base(None)));
    }

    #[test]
    fn clear_removes_state_and_base() {
        let (_dir, store) = store();
        store.clear().unwrap(); // no folder yet
        store.save(&state(1)).unwrap();
        store.save_base(&base(None)).unwrap();
        fs::write(store.dir.join("sync.json.0123456789abcdef.tmp"), b"x").unwrap();
        fs::write(store.dir.join("known_hosts.confirmed"), b"keep").unwrap();
        store.clear().unwrap();
        store.clear().unwrap(); // absent files are fine
        assert_eq!(store.load().unwrap(), None);
        assert_eq!(store.load_base().unwrap(), None);
        let names: Vec<_> = fs::read_dir(&store.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["known_hosts.confirmed"]);
    }

    #[test]
    fn saves_leave_no_temp_file() {
        let (_dir, store) = store();
        store.save(&state(1)).unwrap();
        store.save(&state(2)).unwrap();
        store.save_base(&base(None)).unwrap();
        let mut names: Vec<_> = fs::read_dir(&store.dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [BASE_FILE, STATE_FILE]);
    }
}
