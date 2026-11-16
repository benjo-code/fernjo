//! Tauri commands for terminal sessions and their settings.
//!
//! Like `commands.rs`: plain functions over `&AppState` (testable without Tauri) plus thin
//! wrappers in [`handlers`].

use fernjo_core::{
    AppSettings, DEFAULT_FONT_SIZE, DEFAULT_SCROLLBACK, DataDirSource, TerminalPrefs, settings,
};
use serde::{Deserialize, Serialize};

use crate::commands::CommandError;
use crate::state::AppState;

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

/// The terminal settings with their defaults filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettingsInfo {
    /// `None` means the UI's default monospace stack.
    pub font_family: Option<String>,
    pub font_size: u16,
    pub scrollback: u32,
    /// `false` when the values only live in memory (`FERNJO_DATA_DIR` is set, `settings.json`
    /// is unusable or there is no settings folder).
    pub persistent: bool,
}

/// What the settings dialog sends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettingsInput {
    #[serde(default)]
    pub font_family: Option<String>,
    pub font_size: u16,
    pub scrollback: u32,
}

fn persistent(state: &AppState) -> bool {
    state.source != DataDirSource::Env && !state.settings_unusable && state.config_dir.is_some()
}

fn info(state: &AppState, prefs: &TerminalPrefs) -> TerminalSettingsInfo {
    TerminalSettingsInfo {
        font_family: prefs.font_family.clone(),
        font_size: prefs.font_size.unwrap_or(DEFAULT_FONT_SIZE),
        scrollback: prefs.scrollback.unwrap_or(DEFAULT_SCROLLBACK),
        persistent: persistent(state),
    }
}

pub fn get_terminal_settings(state: &AppState) -> Result<TerminalSettingsInfo, CommandError> {
    let prefs = state
        .terminal
        .lock()
        .map_err(|_| CommandError::internal())?;
    Ok(info(state, &prefs))
}

/// Validates and applies the settings; saves them to `settings.json` (keeping the other
/// fields) unless that file must not be written. Nothing changes on an error.
pub fn set_terminal_settings(
    state: &AppState,
    input: TerminalSettingsInput,
) -> Result<TerminalSettingsInfo, CommandError> {
    let prefs = TerminalPrefs {
        font_family: input
            .font_family
            .map(|f| f.trim().to_string())
            .filter(|f| !f.is_empty()),
        font_size: Some(input.font_size),
        scrollback: Some(input.scrollback),
    };
    prefs.validate().map_err(|code| {
        let message = match code {
            "invalid_font_size" => "the font size must be between 8 and 32",
            _ => "the scrollback must be between 1000 and 100000 lines",
        };
        error(code, message)
    })?;
    let mut slot = state
        .terminal
        .lock()
        .map_err(|_| CommandError::internal())?;
    if let (true, Some(config_dir)) = (persistent(state), &state.config_dir) {
        let path = config_dir.join(settings::FILE_NAME);
        let mut current: AppSettings = settings::load(&path).map_err(|e| {
            error(
                "settings_unusable",
                format!("settings.json cannot be used: {e}"),
            )
        })?;
        current.terminal = Some(prefs.clone());
        settings::save(&path, &current).map_err(|e| {
            error(
                "settings_write_failed",
                format!("could not save the terminal settings: {e}"),
            )
        })?;
    }
    *slot = prefs;
    Ok(info(state, &slot))
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn get_terminal_settings(
        state: State<'_, AppState>,
    ) -> Result<TerminalSettingsInfo, CommandError> {
        super::get_terminal_settings(&state)
    }

    #[tauri::command]
    pub fn set_terminal_settings(
        state: State<'_, AppState>,
        settings: TerminalSettingsInput,
    ) -> Result<TerminalSettingsInfo, CommandError> {
        super::set_terminal_settings(&state, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn input(size: u16, scrollback: u32) -> TerminalSettingsInput {
        TerminalSettingsInput {
            font_family: Some("Fira Code".into()),
            font_size: size,
            scrollback,
        }
    }

    fn settings_path(config: &Path) -> PathBuf {
        config.join(settings::FILE_NAME)
    }

    #[test]
    fn set_then_get_round_trips() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let fresh = get_terminal_settings(&st).unwrap();
        assert_eq!(fresh.font_size, 13);
        assert_eq!(fresh.scrollback, 10_000);
        assert_eq!(fresh.font_family, None);
        let set = set_terminal_settings(&st, input(16, 20_000)).unwrap();
        assert_eq!(set, get_terminal_settings(&st).unwrap());
        assert_eq!(set.font_family.as_deref(), Some("Fira Code"));
        assert_eq!(
            (set.font_size, set.scrollback, set.persistent),
            (16, 20_000, true)
        );
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.terminal.unwrap().font_size, Some(16));
    }

    #[test]
    fn invalid_values_are_refused_and_nothing_changes() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        let before = set_terminal_settings(&st, input(15, 2000)).unwrap();
        let file = std::fs::read(settings_path(config.path())).unwrap();
        for (size, scrollback, code) in [
            (7, 2000, "invalid_font_size"),
            (33, 2000, "invalid_font_size"),
            (15, 999, "invalid_scrollback"),
            (15, 100_001, "invalid_scrollback"),
        ] {
            let err = set_terminal_settings(&st, input(size, scrollback)).unwrap_err();
            assert_eq!(err.code, code);
        }
        assert_eq!(get_terminal_settings(&st).unwrap(), before);
        assert_eq!(std::fs::read(settings_path(config.path())).unwrap(), file);
    }

    #[test]
    fn under_data_dir_env_the_values_apply_but_are_not_persistent_and_no_file_is_written() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Env);
        let set = set_terminal_settings(&st, input(18, 3000)).unwrap();
        assert!(!set.persistent);
        let got = get_terminal_settings(&st).unwrap();
        assert_eq!(
            (got.font_size, got.scrollback, got.persistent),
            (18, 3000, false)
        );
        assert_eq!(std::fs::read_dir(config.path()).unwrap().count(), 0);
    }

    #[test]
    fn persisted_value_keeps_data_dir_in_settings_json() {
        let [data, config, elsewhere] = dirs();
        settings::save(
            &settings_path(config.path()),
            &AppSettings {
                data_dir: Some(elsewhere.path().to_path_buf()),
                terminal: None,
            },
        )
        .unwrap();
        let st = state(data.path(), config.path(), DataDirSource::Settings);
        set_terminal_settings(&st, input(20, 4000)).unwrap();
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.data_dir.as_deref(), Some(elsewhere.path()));
        assert_eq!(saved.terminal.unwrap().scrollback, Some(4000));
    }

    #[test]
    fn unusable_settings_file_is_never_saved_over() {
        let [data, config] = dirs();
        let text = r#"{"schemaVersion":9,"dataDir":"/x"}"#;
        std::fs::write(settings_path(config.path()), text).unwrap();
        let mut st = state(data.path(), config.path(), DataDirSource::Default);
        st.settings_unusable = true;
        let set = set_terminal_settings(&st, input(12, 2000)).unwrap();
        assert!(!set.persistent);
        assert_eq!(get_terminal_settings(&st).unwrap().font_size, 12);
        assert_eq!(
            std::fs::read_to_string(settings_path(config.path())).unwrap(),
            text
        );
        assert_eq!(std::fs::read_dir(config.path()).unwrap().count(), 1);
    }
}
