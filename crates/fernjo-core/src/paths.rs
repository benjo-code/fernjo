//! Where fernjo keeps its data.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::error::LoadError;
use crate::settings;

/// Environment variable that overrides the data folder (tests, the 500-connection check).
pub const DATA_DIR_ENV: &str = "FERNJO_DATA_DIR";

/// Which rule picked the data folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataDirSource {
    /// `FERNJO_DATA_DIR`; `settings.json` was not read.
    Env,
    /// The `dataDir` of `settings.json`.
    Settings,
    /// `<OS config dir>/fernjo`.
    Default,
}

/// The resolved folders of this run.
#[derive(Debug)]
pub struct Location {
    /// Holds `connections.json` and the vault.
    pub data_dir: PathBuf,
    /// Holds `settings.json`: `<OS config dir>/fernjo`. When `FERNJO_DATA_DIR` is set but the
    /// OS has no config directory, it is the same folder as `data_dir` (nothing is read from
    /// or written to it while the variable is set).
    pub config_dir: PathBuf,
    pub source: DataDirSource,
    /// `settings.json` could not be used (unreadable and no usable backup, or a newer schema).
    /// The default folder was used instead; the file is untouched and must not be saved over.
    pub settings_error: Option<LoadError>,
}

/// Resolves the folders. Precedence: a non-empty `env` value, then the absolute `dataDir` of
/// `<config_base>/fernjo/settings.json`, then `<config_base>/fernjo`. A relative or empty
/// `dataDir` counts as unset. `None` only if no folder can be named at all.
pub fn locate_from(env: Option<OsString>, config_base: Option<PathBuf>) -> Option<Location> {
    let config_dir = config_base.map(|base| base.join("fernjo"));
    if let Some(value) = env.filter(|v| !v.is_empty()) {
        let data_dir = PathBuf::from(value);
        return Some(Location {
            config_dir: config_dir.unwrap_or_else(|| data_dir.clone()),
            data_dir,
            source: DataDirSource::Env,
            settings_error: None,
        });
    }
    let config_dir = config_dir?;
    let (custom, settings_error) = match settings::load(&config_dir.join(settings::FILE_NAME)) {
        Ok(s) => (s.data_dir.filter(|d| d.is_absolute()), None),
        Err(e) => (None, Some(e)),
    };
    let (data_dir, source) = match custom {
        Some(dir) => (dir, DataDirSource::Settings),
        None => (config_dir.clone(), DataDirSource::Default),
    };
    Some(Location {
        data_dir,
        config_dir,
        source,
        settings_error,
    })
}

/// The folders for this machine, or `None` if the OS gives no config directory (and no
/// `FERNJO_DATA_DIR` is set).
pub fn locate() -> Option<Location> {
    locate_from(
        std::env::var_os(DATA_DIR_ENV),
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().to_path_buf()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{self, AppSettings};
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn write_settings(base: &Path, text: &str) {
        let dir = base.join("fernjo");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(settings::FILE_NAME), text).unwrap();
    }

    #[test]
    fn env_wins_over_settings_and_settings_is_not_read() {
        let base = tempfile::tempdir().unwrap();
        // A read would recover from the valid .bak and move the corrupt file aside, which is
        // visible on disk; so an untouched folder proves settings.json is never opened.
        write_settings(base.path(), "{ not json");
        let dir = base.path().join("fernjo");
        let good = r#"{"schemaVersion":1,"dataDir":"/elsewhere"}"#;
        fs::write(dir.join("settings.json.bak"), good).unwrap();
        let listing = || {
            let mut names: Vec<String> = fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        };
        let before = listing();
        let loc = locate_from(
            Some(OsString::from("/tmp/custom")),
            Some(base.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(loc.data_dir, PathBuf::from("/tmp/custom"));
        assert_eq!(loc.source, DataDirSource::Env);
        assert_eq!(loc.config_dir, base.path().join("fernjo"));
        assert!(loc.settings_error.is_none());
        assert_eq!(listing(), before);
        assert!(listing().iter().all(|n| !n.contains(".corrupt-")));
        assert_eq!(
            fs::read_to_string(dir.join(settings::FILE_NAME)).unwrap(),
            "{ not json"
        );
    }

    #[test]
    fn settings_data_dir_wins_over_default() {
        let base = tempfile::tempdir().unwrap();
        let target = std::env::temp_dir().join("fernjo-elsewhere");
        settings::save(
            &base.path().join("fernjo").join(settings::FILE_NAME),
            &AppSettings {
                data_dir: Some(target.clone()),
                terminal: None,
            },
        )
        .unwrap();
        let loc = locate_from(Some(OsString::new()), Some(base.path().to_path_buf())).unwrap();
        assert_eq!(loc.data_dir, target);
        assert_eq!(loc.source, DataDirSource::Settings);
        assert_eq!(loc.config_dir, base.path().join("fernjo"));
    }

    #[test]
    fn default_is_config_dir_fernjo() {
        let base = tempfile::tempdir().unwrap();
        let loc = locate_from(None, Some(base.path().to_path_buf())).unwrap();
        assert_eq!(loc.data_dir, base.path().join("fernjo"));
        assert_eq!(loc.config_dir, loc.data_dir);
        assert_eq!(loc.source, DataDirSource::Default);
        assert!(loc.settings_error.is_none());
    }

    #[test]
    fn no_config_dir_and_no_env_is_none() {
        assert!(locate_from(None, None).is_none());
        assert!(locate_from(Some(OsString::new()), None).is_none());
    }

    #[test]
    fn env_without_config_dir_keeps_the_override_usable() {
        let loc = locate_from(Some(OsString::from("/tmp/custom")), None).unwrap();
        assert_eq!(loc.data_dir, PathBuf::from("/tmp/custom"));
        assert_eq!(loc.config_dir, PathBuf::from("/tmp/custom"));
        assert_eq!(loc.source, DataDirSource::Env);
    }

    #[test]
    fn relative_or_empty_settings_data_dir_is_treated_as_unset() {
        for text in [
            r#"{"schemaVersion":1,"dataDir":"relative/dir"}"#,
            r#"{"schemaVersion":1,"dataDir":""}"#,
        ] {
            let base = tempfile::tempdir().unwrap();
            write_settings(base.path(), text);
            let loc = locate_from(None, Some(base.path().to_path_buf())).unwrap();
            assert_eq!(loc.source, DataDirSource::Default, "{text}");
            assert_eq!(loc.data_dir, base.path().join("fernjo"));
            assert!(loc.settings_error.is_none());
        }
    }

    #[test]
    fn unreadable_settings_fall_back_to_default_and_report_the_error() {
        let base = tempfile::tempdir().unwrap();
        write_settings(base.path(), "{ not json");
        let loc = locate_from(None, Some(base.path().to_path_buf())).unwrap();
        assert_eq!(loc.source, DataDirSource::Default);
        assert_eq!(loc.data_dir, base.path().join("fernjo"));
        assert!(matches!(
            loc.settings_error,
            Some(crate::LoadError::Unreadable { .. })
        ));
    }

    #[test]
    fn newer_settings_fall_back_to_default_report_and_stay_untouched() {
        let base = tempfile::tempdir().unwrap();
        let text = r#"{"schemaVersion":9,"dataDir":"/x"}"#;
        write_settings(base.path(), text);
        let loc = locate_from(None, Some(base.path().to_path_buf())).unwrap();
        assert_eq!(loc.source, DataDirSource::Default);
        assert!(matches!(
            loc.settings_error,
            Some(crate::LoadError::NewerSchema { found: 9, .. })
        ));
        let file = base.path().join("fernjo").join(settings::FILE_NAME);
        assert_eq!(fs::read_to_string(file).unwrap(), text);
    }
}
