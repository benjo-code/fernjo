//! Tauri commands for app-wide settings that are not terminal settings (the colour theme).
//!
//! Like `commands.rs`: plain functions over `&AppState` plus thin wrappers in [`handlers`].

use fernjo_core::{AppSettings, Theme, settings};
use serde::Serialize;

use crate::commands::CommandError;
use crate::session_commands::{UnsavedReason, persistent, unsaved_reason};
use crate::state::AppState;

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// The theme choice and whether it is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeInfo {
    pub theme: Theme,
    /// `false` when the value only lives in memory (`FERNJO_DATA_DIR` is set, `settings.json`
    /// is unusable or there is no settings folder).
    pub persistent: bool,
    /// Why the value is not saved; `None` when it is.
    pub unsaved_reason: Option<UnsavedReason>,
}

/// Loads `settings.json`, applies `change` and saves it, keeping every other field. Does
/// nothing when the file must not be written (see `persistent`).
///
/// # Errors
/// `settings_unusable` when the file cannot be loaded, `settings_write_failed` when saving fails.
pub(crate) fn update_settings(
    state: &AppState,
    change: impl FnOnce(&mut AppSettings),
) -> Result<(), CommandError> {
    if let (true, Some(config_dir)) = (persistent(state), &state.config_dir) {
        let path = config_dir.join(settings::FILE_NAME);
        let mut current: AppSettings = settings::load(&path).map_err(|e| {
            error(
                "settings_unusable",
                format!("settings.json cannot be used: {e}"),
            )
        })?;
        change(&mut current);
        settings::save(&path, &current).map_err(|e| {
            error(
                "settings_write_failed",
                format!("could not save the settings: {e}"),
            )
        })?;
    }
    Ok(())
}

pub fn get_theme(state: &AppState) -> Result<ThemeInfo, CommandError> {
    let theme = *state.theme.lock().map_err(|_| CommandError::internal())?;
    Ok(ThemeInfo {
        theme,
        persistent: persistent(state),
        unsaved_reason: unsaved_reason(state),
    })
}

/// Applies the theme and saves it to `settings.json` (keeping the other fields) unless that
/// file must not be written. Nothing changes on an error.
pub fn set_theme(state: &AppState, theme: Theme) -> Result<ThemeInfo, CommandError> {
    let mut slot = state.theme.lock().map_err(|_| CommandError::internal())?;
    update_settings(state, |s| s.theme = Some(theme))?;
    *slot = theme;
    Ok(ThemeInfo {
        theme,
        persistent: persistent(state),
        unsaved_reason: unsaved_reason(state),
    })
}

pub mod handlers {
    use tauri::State;

    use super::{CommandError, Theme, ThemeInfo};
    use crate::state::AppState;

    #[tauri::command]
    pub fn get_theme(state: State<'_, AppState>) -> Result<ThemeInfo, CommandError> {
        super::get_theme(&state)
    }

    #[tauri::command]
    pub fn set_theme(state: State<'_, AppState>, theme: Theme) -> Result<ThemeInfo, CommandError> {
        super::set_theme(&state, theme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_commands::{TerminalSettingsInput, set_terminal_settings};
    use fernjo_core::{AppSettings, DataDirSource, Location, settings};
    use std::path::{Path, PathBuf};

    fn dirs<const N: usize>() -> [tempfile::TempDir; N] {
        std::array::from_fn(|_| tempfile::tempdir().unwrap())
    }

    fn state(data: &Path, config: &Path, source: DataDirSource) -> AppState {
        let mut st = AppState::open(Some(data.to_path_buf()));
        st.config_dir = Some(config.to_path_buf());
        st.source = source;
        st
    }

    fn settings_path(config: &Path) -> PathBuf {
        config.join(settings::FILE_NAME)
    }

    #[test]
    fn set_theme_persists_and_keeps_data_dir_and_terminal() {
        let [data, config, elsewhere] = dirs();
        settings::save(
            &settings_path(config.path()),
            &AppSettings {
                data_dir: Some(elsewhere.path().to_path_buf()),
                ..AppSettings::default()
            },
        )
        .unwrap();
        let st = state(data.path(), config.path(), DataDirSource::Settings);
        set_terminal_settings(
            &st,
            TerminalSettingsInput {
                font_family: None,
                font_size: 15,
                scrollback: 2000,
            },
        )
        .unwrap();
        let info = set_theme(&st, Theme::Dark).unwrap();
        assert_eq!(
            info,
            ThemeInfo {
                theme: Theme::Dark,
                persistent: true,
                unsaved_reason: None
            }
        );
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.theme, Some(Theme::Dark));
        assert_eq!(saved.data_dir.as_deref(), Some(elsewhere.path()));
        assert_eq!(saved.terminal.unwrap().font_size, Some(15));
    }

    #[test]
    fn set_theme_survives_reopening_the_state() {
        let [data, config] = dirs();
        let open = || {
            AppState::open_for_app(Some(Location {
                data_dir: data.path().to_path_buf(),
                config_dir: config.path().to_path_buf(),
                source: DataDirSource::Default,
                settings_error: None,
                device_dir: None,
            }))
        };
        let st = open();
        assert_eq!(get_theme(&st).unwrap().theme, Theme::System);
        set_theme(&st, Theme::Light).unwrap();
        drop(st);
        assert_eq!(
            get_theme(&open()).unwrap(),
            ThemeInfo {
                theme: Theme::Light,
                persistent: true,
                unsaved_reason: None
            }
        );
    }

    #[test]
    fn set_theme_under_the_env_override_is_memory_only_and_writes_nothing() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Env);
        let info = set_theme(&st, Theme::Dark).unwrap();
        assert!(!info.persistent);
        assert_eq!(info.unsaved_reason, Some(UnsavedReason::Env));
        assert_eq!(get_theme(&st).unwrap().theme, Theme::Dark);
        assert!(!settings_path(config.path()).exists());
    }

    #[test]
    fn set_theme_with_unusable_settings_is_refused_and_the_file_untouched() {
        let [data, config] = dirs();
        let text = r#"{"schemaVersion":9,"dataDir":"/x"}"#;
        std::fs::write(settings_path(config.path()), text).unwrap();
        let mut st = state(data.path(), config.path(), DataDirSource::Default);
        st.settings_unusable = true;
        // Not persistent: applied in memory only, the file is never written.
        let info = set_theme(&st, Theme::Dark).unwrap();
        assert!(!info.persistent);
        assert_eq!(info.unsaved_reason, Some(UnsavedReason::SettingsUnusable));
        assert_eq!(
            std::fs::read_to_string(settings_path(config.path())).unwrap(),
            text
        );

        // The file turned unusable behind the app's back: the write is refused.
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let err = set_theme(&st, Theme::Light).unwrap_err();
        assert_eq!(err.code, "settings_unusable");
        assert_eq!(get_theme(&st).unwrap().theme, Theme::System);
        assert_eq!(
            std::fs::read_to_string(settings_path(config.path())).unwrap(),
            text
        );
    }

    #[test]
    fn set_terminal_settings_still_keeps_the_theme() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        set_theme(&st, Theme::Dark).unwrap();
        set_terminal_settings(
            &st,
            TerminalSettingsInput {
                font_family: None,
                font_size: 14,
                scrollback: 3000,
            },
        )
        .unwrap();
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.theme, Some(Theme::Dark));
        assert_eq!(saved.terminal.unwrap().font_size, Some(14));
    }
}
