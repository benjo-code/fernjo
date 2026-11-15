//! Where fernjo keeps its data.

use std::ffi::OsString;
use std::path::PathBuf;

/// Environment variable that overrides the data folder (tests, the 500-connection check).
pub const DATA_DIR_ENV: &str = "FERNJO_DATA_DIR";

/// Picks the data folder: a non-empty `env` value wins, otherwise `<config_dir>/fernjo`.
pub fn data_dir_from(env: Option<OsString>, config_dir: Option<PathBuf>) -> Option<PathBuf> {
    match env {
        Some(value) if !value.is_empty() => Some(PathBuf::from(value)),
        _ => config_dir.map(|dir| dir.join("fernjo")),
    }
}

/// The data folder for this machine, or `None` if the OS gives no config directory.
pub fn data_dir() -> Option<PathBuf> {
    data_dir_from(
        std::env::var_os(DATA_DIR_ENV),
        directories::BaseDirs::new().map(|dirs| dirs.config_dir().to_path_buf()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn data_dir_from_prefers_non_empty_env() {
        let dir = data_dir_from(
            Some(OsString::from("/tmp/custom")),
            Some(PathBuf::from("/home/x/.config")),
        );
        assert_eq!(dir, Some(PathBuf::from("/tmp/custom")));
    }

    #[test]
    fn data_dir_from_appends_fernjo_to_config_dir() {
        let dir = data_dir_from(None, Some(PathBuf::from("/home/x/.config")));
        assert_eq!(dir, Some(PathBuf::from("/home/x/.config").join("fernjo")));
    }

    #[test]
    fn data_dir_from_empty_env_falls_back() {
        let dir = data_dir_from(Some(OsString::new()), Some(PathBuf::from("/c")));
        assert_eq!(dir, Some(PathBuf::from("/c").join("fernjo")));
    }

    #[test]
    fn data_dir_from_nothing_is_none() {
        assert_eq!(data_dir_from(None, None), None);
        assert_eq!(data_dir_from(Some(OsString::new()), None), None);
    }
}
