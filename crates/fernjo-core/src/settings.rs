//! `settings.json`: the small per-machine settings file in the OS config folder.
//!
//! It lives in `<OS config dir>/fernjo/`, never in the data folder (it says where the data
//! folder is). Same rules as `connections.json`: atomic write with `.bak`, missing means
//! defaults, a newer schema is refused and never overwritten.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::{self, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{LoadError, SaveError};
use crate::json;
use crate::persist::{move_aside, read_capped, with_suffix, write_atomic};

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
    /// Terminal appearance; unset means the built-in defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalPrefs>,
    /// Colour theme; unset means follow the system. An unknown value loads as unset.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "lenient_theme"
    )]
    pub theme: Option<Theme>,
    /// Lock the vault after this many idle minutes; unset means never.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_auto_lock_minutes: Option<u32>,
    /// Reopen the tabs of the last session on start; unset means on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reopen_tabs: Option<bool>,
}

/// Allowed idle times for the vault auto-lock, in minutes.
pub const AUTO_LOCK_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 1..=1440;

/// Checks the auto-lock minutes (`None` means off).
///
/// # Errors
/// `invalid_auto_lock` when a value is outside [`AUTO_LOCK_MINUTES_RANGE`].
pub fn validate_auto_lock(minutes: Option<u32>) -> Result<(), &'static str> {
    match minutes {
        Some(m) if !AUTO_LOCK_MINUTES_RANGE.contains(&m) => Err("invalid_auto_lock"),
        _ => Ok(()),
    }
}

/// The colour theme choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the operating system.
    #[default]
    System,
    Light,
    Dark,
}

/// Reads the theme, mapping an unknown string or a wrong JSON type to `None` so a hand-edited
/// or newer value never makes the whole file unusable. A value that is not a string is skipped
/// without being kept (M11 review #28).
fn lenient_theme<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Theme>, D::Error> {
    d.deserialize_any(LenientTheme)
}

struct LenientTheme;

impl<'de> Visitor<'de> for LenientTheme {
    type Value = Option<Theme>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Option<Theme>, E> {
        Ok(match v {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        })
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Option<Theme>, E> {
        Ok(None)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Option<Theme>, E> {
        Ok(None)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Option<Theme>, E> {
        Ok(None)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Option<Theme>, E> {
        Ok(None)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Option<Theme>, E> {
        Ok(None)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Option<Theme>, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(None)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Option<Theme>, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(None)
    }
}

/// Default terminal font size in pixels.
pub const DEFAULT_FONT_SIZE: u16 = 13;
/// Whether a tab closes itself when its shell ended cleanly, unless set.
pub const DEFAULT_CLOSE_ON_EXIT: bool = true;
/// Default number of scrollback lines.
pub const DEFAULT_SCROLLBACK: u32 = 10_000;
/// Allowed terminal font sizes.
pub const FONT_SIZE_RANGE: std::ops::RangeInclusive<u16> = 8..=32;
/// Allowed scrollback line counts.
pub const SCROLLBACK_RANGE: std::ops::RangeInclusive<u32> = 1000..=100_000;

/// Terminal preferences; an unset field means its default.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalPrefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scrollback: Option<u32>,
    /// Close the tab when the remote shell ended cleanly; unset means [`DEFAULT_CLOSE_ON_EXIT`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_on_exit: Option<bool>,
}

impl TerminalPrefs {
    /// Checks the set fields against their ranges.
    ///
    /// # Errors
    /// The error code: `invalid_font_size` or `invalid_scrollback`.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self
            .font_size
            .is_some_and(|v| !FONT_SIZE_RANGE.contains(&v))
        {
            return Err("invalid_font_size");
        }
        if self
            .scrollback
            .is_some_and(|v| !SCROLLBACK_RANGE.contains(&v))
        {
            return Err("invalid_scrollback");
        }
        Ok(())
    }
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
    let bytes = match read_capped(path) {
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
    // validated and `schemaVersion` read without building a `serde_json::Value` (`crate::json`)
    let version = match json::head(bytes, ["schemaVersion"]) {
        Ok([version]) => version,
        Err(e) => return Ok(Read::Bad(e.to_string())),
    };
    let Some(version) = version.as_u64() else {
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
    match serde_json::from_slice::<AppSettings>(bytes) {
        Ok(settings) => Ok(Read::Parsed(settings)),
        Err(e) => Ok(Read::Bad(e.to_string())),
    }
}

/// Loads `path`: a missing file gives the defaults, a newer schema is refused, an unreadable
/// main file falls back to `.bak` (the bad file is moved aside) and otherwise fails with
/// [`LoadError::Unreadable`]. Nothing else is written.
pub fn load(path: &Path) -> Result<AppSettings, LoadError> {
    // leftovers of an interrupted save (`settings.json.<16 hex>.tmp`, best effort)
    fernjo_fs::sweep_stale_temps(path, fernjo_fs::STALE_TEMP_AGE);
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

    fn read_json(dir: &Path) -> serde_json::Value {
        serde_json::from_slice(&fs::read(path(dir)).unwrap()).unwrap()
    }

    fn with_dir(p: &str) -> AppSettings {
        AppSettings {
            data_dir: Some(PathBuf::from(p)),
            terminal: None,
            theme: None,
            vault_auto_lock_minutes: None,
            reopen_tabs: None,
        }
    }

    #[test]
    fn settings_load_refuses_a_file_over_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let p = path(dir.path());
        let file = fs::File::create(&p).unwrap();
        file.set_len(crate::persist::MAX_DATA_FILE_LEN + 1).unwrap();
        drop(file);
        assert!(matches!(load(&p), Err(LoadError::Unreadable { .. })));
        assert_eq!(
            fs::metadata(&p).unwrap().len(),
            crate::persist::MAX_DATA_FILE_LEN + 1
        );
    }

    /// Final M11 review, Minor 2: a temp file of an interrupted save older than a day is removed
    /// on open; a younger one stays (it may belong to a save that is still running).
    #[test]
    fn load_removes_stale_temp_files_of_settings_json() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join(format!("{FILE_NAME}.0123456789abcdef.tmp"));
        let fresh = dir.path().join(format!("{FILE_NAME}.fedcba9876543210.tmp"));
        for p in [&stale, &fresh] {
            fs::write(p, b"x").unwrap();
        }
        let old = fernjo_fs::STALE_TEMP_AGE + std::time::Duration::from_secs(3600);
        fs::OpenOptions::new()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(std::time::SystemTime::now() - old)
            .unwrap();
        load(&path(dir.path())).unwrap();
        assert!(!stale.exists());
        assert!(fresh.exists());
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
    fn terminal_prefs_round_trip_and_leave_data_dir_alone() {
        let dir = tempfile::tempdir().unwrap();
        let settings = AppSettings {
            data_dir: Some(PathBuf::from("/srv/fernjo")),
            terminal: Some(TerminalPrefs {
                font_family: Some("Fira Code".into()),
                font_size: Some(15),
                scrollback: Some(5000),
                close_on_exit: Some(false),
            }),
            theme: None,
            vault_auto_lock_minutes: None,
            reopen_tabs: None,
        };
        save(&path(dir.path()), &settings).unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), settings);
        let v: serde_json::Value =
            serde_json::from_slice(&fs::read(path(dir.path())).unwrap()).unwrap();
        assert_eq!(
            v,
            json!({"schemaVersion": 1, "dataDir": "/srv/fernjo",
                   "terminal": {"fontFamily": "Fira Code", "fontSize": 15, "scrollback": 5000,
                                "closeOnExit": false}})
        );
    }

    #[test]
    fn validate_rejects_font_size_7_and_33_and_scrollback_999_and_100001() {
        let p = |font_size, scrollback| TerminalPrefs {
            font_family: None,
            font_size,
            scrollback,
            close_on_exit: None,
        };
        assert_eq!(p(Some(7), None).validate(), Err("invalid_font_size"));
        assert_eq!(p(Some(33), None).validate(), Err("invalid_font_size"));
        assert_eq!(p(None, Some(999)).validate(), Err("invalid_scrollback"));
        assert_eq!(p(None, Some(100_001)).validate(), Err("invalid_scrollback"));
        assert_eq!(p(Some(8), Some(1000)).validate(), Ok(()));
        assert_eq!(p(Some(32), Some(100_000)).validate(), Ok(()));
        assert_eq!(p(None, None).validate(), Ok(()));
    }

    #[test]
    fn settings_without_terminal_still_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(path(dir.path()), r#"{"schemaVersion":1,"dataDir":"/x"}"#).unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), with_dir("/x"));
    }

    #[test]
    fn settings_without_close_on_exit_still_load() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            path(dir.path()),
            r#"{"schemaVersion":1,"terminal":{"fontSize":14,"scrollback":2000}}"#,
        )
        .unwrap();
        let terminal = load(&path(dir.path())).unwrap().terminal.unwrap();
        assert_eq!(terminal.font_size, Some(14));
        assert_eq!(terminal.close_on_exit, None);
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

    #[test]
    fn theme_round_trips_through_the_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let settings = AppSettings {
            theme: Some(Theme::Dark),
            ..AppSettings::default()
        };
        save(&path(dir.path()), &settings).unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), settings);
        assert_eq!(
            read_json(dir.path()),
            json!({"schemaVersion": 1, "theme": "dark"})
        );
    }

    #[test]
    fn unset_theme_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        save(&path(dir.path()), &AppSettings::default()).unwrap();
        assert_eq!(read_json(dir.path()), json!({"schemaVersion": 1}));
    }

    #[test]
    fn unknown_theme_value_loads_as_unset_and_keeps_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            path(dir.path()),
            r#"{"schemaVersion":1,"dataDir":"/x","theme":"banana"}"#,
        )
        .unwrap();
        let loaded = load(&path(dir.path())).unwrap();
        assert_eq!(loaded.theme, None);
        assert_eq!(loaded.data_dir.as_deref(), Some(Path::new("/x")));
    }

    #[test]
    fn theme_of_the_wrong_type_loads_as_unset() {
        for bad in ["3", "null", "{}"] {
            let dir = tempfile::tempdir().unwrap();
            fs::write(
                path(dir.path()),
                format!(r#"{{"schemaVersion":1,"theme":{bad}}}"#),
            )
            .unwrap();
            assert_eq!(load(&path(dir.path())).unwrap().theme, None, "{bad}");
        }
    }

    #[test]
    fn auto_lock_defaults_to_off_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        // an old file without the field still loads, as off
        fs::write(path(dir.path()), r#"{"schemaVersion":1,"dataDir":"/x"}"#).unwrap();
        assert_eq!(
            load(&path(dir.path())).unwrap().vault_auto_lock_minutes,
            None
        );
        save(&path(dir.path()), &AppSettings::default()).unwrap();
        assert_eq!(read_json(dir.path()), json!({"schemaVersion": 1}));
        let settings = AppSettings {
            vault_auto_lock_minutes: Some(15),
            ..AppSettings::default()
        };
        save(&path(dir.path()), &settings).unwrap();
        assert_eq!(load(&path(dir.path())).unwrap(), settings);
        assert_eq!(
            read_json(dir.path()),
            json!({"schemaVersion": 1, "vaultAutoLockMinutes": 15})
        );
    }

    #[test]
    fn auto_lock_range_is_1_to_1440() {
        assert_eq!(validate_auto_lock(Some(0)), Err("invalid_auto_lock"));
        assert_eq!(validate_auto_lock(Some(1441)), Err("invalid_auto_lock"));
        assert_eq!(validate_auto_lock(Some(1)), Ok(()));
        assert_eq!(validate_auto_lock(Some(1440)), Ok(()));
        assert_eq!(validate_auto_lock(None), Ok(()));
    }
}
