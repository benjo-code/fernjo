//! `settings.json`: the small per-machine settings file in the OS config folder.
//!
//! It lives in `<OS config dir>/fernjo/`, never in the data folder (it says where the data
//! folder is). Same rules as `connections.json`: atomic write with `.bak`, missing means
//! defaults, a newer schema is refused and never overwritten.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{LoadError, SaveError};
use crate::persist::{move_aside, with_suffix, write_atomic};

/// Name of the settings file inside the config folder.
pub const FILE_NAME: &str = "settings.json";
/// File format version this build writes and understands.
pub const CURRENT_VERSION: u32 = 1;

/// The settings. Unknown fields of the same schema version are ignored on load.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// Custom data folder; unset means `<OS config dir>/fernjo`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<PathBuf>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileOut<'a> {
    schema_version: u32,
    #[serde(flatten)]
    settings: &'a AppSettings,
}

enum Read {
    Missing,
    Parsed(AppSettings),
    Bad(String),
}

fn read_file(path: &Path) -> Result<Read, LoadError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Read::Missing),
        Err(e) => {
            return Err(LoadError::Unreadable {
                path: path.to_path_buf(),
                reason: e.to_string(),
            });
        }
    };
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes);
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return Ok(Read::Bad(e.to_string())),
    };
    let Some(version) = value.get("schemaVersion").and_then(Value::as_u64) else {
        return Ok(Read::Bad("missing or invalid schemaVersion".to_string()));
    };
    let version = u32::try_from(version).unwrap_or(u32::MAX);
    if version > CURRENT_VERSION {
        return Err(LoadError::NewerSchema {
            found: version,
            supported: CURRENT_VERSION,
            path: path.to_path_buf(),
        });
    }
    if version == 0 {
        return Ok(Read::Bad("invalid schemaVersion 0".to_string()));
    }
    match serde_json::from_value::<AppSettings>(value) {
        Ok(settings) => Ok(Read::Parsed(settings)),
        Err(e) => Ok(Read::Bad(e.to_string())),
    }
}

/// Loads `path`: a missing file gives the defaults, a newer schema is refused, an unreadable
/// main file falls back to `.bak` (the bad file is moved aside) and otherwise fails with
/// [`LoadError::Unreadable`]. Nothing else is written.
pub fn load(path: &Path) -> Result<AppSettings, LoadError> {
    let reason = match read_file(path)? {
        Read::Parsed(settings) => return Ok(settings),
        Read::Missing => None,
        Read::Bad(reason) => Some(reason),
    };
    let bak = with_suffix(path, ".bak");
    match read_file(&bak)? {
        Read::Parsed(settings) => {
            if reason.is_some() {
                move_aside(path).map_err(|e| LoadError::Unreadable {
                    path: path.to_path_buf(),
                    reason: format!("could not move the unreadable file aside: {e}"),
                })?;
            }
            Ok(settings)
        }
        Read::Missing => match reason {
            None => Ok(AppSettings::default()),
            Some(reason) => Err(LoadError::Unreadable {
                path: path.to_path_buf(),
                reason,
            }),
        },
        Read::Bad(bak_reason) => Err(LoadError::Unreadable {
            path: path.to_path_buf(),
            reason: match reason {
                Some(r) => format!("{r}; the backup is unreadable too ({bak_reason})"),
                None => format!("the file is missing and the backup is unreadable ({bak_reason})"),
            },
        }),
    }
}

/// Writes `settings` atomically (temp file, flush, `.bak`, rename).
pub fn save(path: &Path, settings: &AppSettings) -> Result<(), SaveError> {
    let json = serde_json::to_string_pretty(&FileOut {
        schema_version: CURRENT_VERSION,
        settings,
    })
    .map_err(|e| SaveError {
        path: path.to_path_buf(),
        source: io::Error::other(e),
    })?;
    write_atomic(path, json.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::LoadError;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;

    fn path(dir: &Path) -> PathBuf {
        dir.join(FILE_NAME)
    }

    fn with_dir(p: &str) -> AppSettings {
        AppSettings {
            data_dir: Some(PathBuf::from(p)),
        }
    }

    #[test]
    fn missing_file_is_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), AppSettings::default());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn round_trip_keeps_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        save(&path(dir.path()), &with_dir("/srv/fernjo")).unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), with_dir("/srv/fernjo"));
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(v, json!({ "schemaVersion": 1, "dataDir": "/srv/fernjo" }));
        save(&path(dir.path()), &AppSettings::default()).unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(v, json!({ "schemaVersion": 1 }));
    }

    #[test]
    fn save_keeps_bak_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        save(&path(dir.path()), &with_dir("/a")).unwrap();
        let first = fs::read(path(dir.path())).unwrap();
        save(&path(dir.path()), &with_dir("/b")).unwrap();
        let bak = dir.path().join("settings.json.bak");
        assert_eq!(fs::read(bak).unwrap(), first);
        let mut names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["settings.json", "settings.json.bak"]);
    }

    #[test]
    fn newer_schema_is_refused_and_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let text = json!({ "schemaVersion": 9, "dataDir": "/x", "theme": "dark" }).to_string();
        fs::write(path(dir.path()), &text).unwrap();
        let err = load(&path(dir.path())).unwrap_err();
        assert!(
            matches!(
                err,
                LoadError::NewerSchema {
                    found: 9,
                    supported: 1,
                    ..
                }
            ),
            "{err:?}"
        );
        assert_eq!(fs::read_to_string(path(dir.path())).unwrap(), text);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_file_falls_back_to_bak() {
        let dir = tempfile::tempdir().unwrap();
        save(&path(dir.path()), &with_dir("/a")).unwrap();
        save(&path(dir.path()), &with_dir("/b")).unwrap();
        fs::write(path(dir.path()), "{ not json").unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), with_dir("/a"));
        // the corrupt file was moved aside, so the next save cannot poison the backup
        assert!(!path(dir.path()).exists());
        let aside = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .any(|n| n.starts_with("settings.json.corrupt-"));
        assert!(aside);
    }

    #[test]
    fn corrupt_file_without_bak_is_unreadable() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(path(dir.path()), "garbage").unwrap();
        let err = load(&path(dir.path())).unwrap_err();
        assert!(matches!(err, LoadError::Unreadable { .. }), "{err:?}");
        assert_eq!(fs::read_to_string(path(dir.path())).unwrap(), "garbage");
    }
}
