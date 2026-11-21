//! Tauri commands for terminal sessions and their settings.
//!
//! Like `commands.rs`: plain functions over `&AppState` (testable without Tauri) plus thin
//! wrappers in [`handlers`].

use fernjo_core::NodeId;
use fernjo_core::{
    DEFAULT_CLOSE_ON_EXIT, DEFAULT_FONT_SIZE, DEFAULT_SCROLLBACK, DataDirSource, TerminalPrefs,
};
use serde::{Deserialize, Serialize};
use tauri::ipc::{Channel, InvokeResponseBody};

use crate::commands::CommandError;
use crate::sessions::PromptAnswer;
use crate::state::AppState;

/// Header of `session_input` carrying the session id (the body is the raw bytes).
pub const SESSION_HEADER: &str = "x-fernjo-session";

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
    /// Close a tab when its remote shell ended cleanly (the UI applies it).
    pub close_on_exit: bool,
    /// `false` when the values only live in memory (`FERNJO_DATA_DIR` is set, `settings.json`
    /// is unusable or there is no settings folder).
    pub persistent: bool,
    /// Why the values are not saved; `None` when they are.
    pub unsaved_reason: Option<UnsavedReason>,
}

/// Why settings only live in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UnsavedReason {
    /// `FERNJO_DATA_DIR` is set.
    Env,
    /// `settings.json` is unreadable or from a newer version.
    SettingsUnusable,
    /// The device has no settings folder.
    NoConfigDir,
}

/// What the settings dialog sends.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettingsInput {
    #[serde(default)]
    pub font_family: Option<String>,
    pub font_size: u16,
    pub scrollback: u32,
    /// An input without the key (an older UI) means the default, `true`.
    #[serde(default = "default_close_on_exit")]
    pub close_on_exit: bool,
}

fn default_close_on_exit() -> bool {
    DEFAULT_CLOSE_ON_EXIT
}

/// Why settings cannot be saved, in order of precedence; `None` when they can.
pub(crate) fn unsaved_reason(state: &AppState) -> Option<UnsavedReason> {
    if state.source == DataDirSource::Env {
        Some(UnsavedReason::Env)
    } else if state.settings_unusable {
        Some(UnsavedReason::SettingsUnusable)
    } else if state.config_dir.is_none() {
        Some(UnsavedReason::NoConfigDir)
    } else {
        None
    }
}

pub(crate) fn persistent(state: &AppState) -> bool {
    unsaved_reason(state).is_none()
}

fn info(state: &AppState, prefs: &TerminalPrefs) -> TerminalSettingsInfo {
    TerminalSettingsInfo {
        font_family: prefs.font_family.clone(),
        font_size: prefs.font_size.unwrap_or(DEFAULT_FONT_SIZE),
        scrollback: prefs.scrollback.unwrap_or(DEFAULT_SCROLLBACK),
        close_on_exit: prefs.close_on_exit.unwrap_or(DEFAULT_CLOSE_ON_EXIT),
        persistent: persistent(state),
        unsaved_reason: unsaved_reason(state),
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
        close_on_exit: Some(input.close_on_exit),
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
    crate::app_settings::update_settings(state, |s| s.terminal = Some(prefs.clone()))?;
    *slot = prefs;
    Ok(info(state, &slot))
}

/// Opens a session for a connection and returns its id (a UUID). Everything is resolved in Rust.
/// For an RDP connection `cols` and `rows` are the size of the tab in pixels.
/// Output (raw) and events (JSON) arrive on `on_event`; see the wire contract in `sessions.rs`.
///
/// # Errors
/// `not_found`, `not_a_connection`, `invalid_host`, `no_data_dir`, or a
/// vault error.
pub fn open_session(
    state: &AppState,
    node_id: NodeId,
    on_event: Channel<InvokeResponseBody>,
    cols: u32,
    rows: u32,
) -> Result<String, CommandError> {
    state.sessions.open(state, node_id, on_event, cols, rows)
}

/// Queues keystrokes. `not_found` for an unknown id, `session_not_connected` without a shell.
pub fn session_input(state: &AppState, id: &str, data: Vec<u8>) -> Result<(), CommandError> {
    state.sessions.input(id, data)
}

pub fn session_resize(
    state: &AppState,
    id: &str,
    cols: u32,
    rows: u32,
) -> Result<(), CommandError> {
    state.sessions.resize(id, cols, rows)
}

/// The UI finished writing `bytes` of output to the terminal (flow control).
pub fn session_ack(state: &AppState, id: &str, bytes: usize) -> Result<(), CommandError> {
    state.sessions.ack(id, bytes)
}

pub fn close_session(state: &AppState, id: &str) -> Result<(), CommandError> {
    state.sessions.close(id)
}

/// Connects again; only when the session is disconnected, else `session_active`.
pub fn reconnect_session(
    state: &AppState,
    id: &str,
    cols: u32,
    rows: u32,
) -> Result<(), CommandError> {
    state.sessions.reconnect(state, id, cols, rows)
}

pub fn answer_prompt(
    state: &AppState,
    id: &str,
    prompt_id: u64,
    answer: PromptAnswer,
) -> Result<(), CommandError> {
    state.sessions.answer(id, prompt_id, answer)
}

/// Continues a session waiting for the vault; only then, else `session_active`.
pub fn resume_session(
    state: &AppState,
    id: &str,
    skip_credential: bool,
) -> Result<(), CommandError> {
    state.sessions.resume(state, id, skip_credential)
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::ipc::{InvokeBody, Request};
    use tauri::{AppHandle, State};
    use tauri_plugin_opener::OpenerExt;

    #[tauri::command]
    pub fn open_session(
        state: State<'_, AppState>,
        node_id: NodeId,
        on_event: Channel<InvokeResponseBody>,
        cols: u32,
        rows: u32,
    ) -> Result<String, CommandError> {
        super::open_session(&state, node_id, on_event, cols, rows)
    }

    /// The body is the raw bytes, the session id travels in the `x-fernjo-session` header.
    #[tauri::command]
    pub fn session_input(
        state: State<'_, AppState>,
        request: Request<'_>,
    ) -> Result<(), CommandError> {
        let InvokeBody::Raw(data) = request.body() else {
            return Err(CommandError {
                code: "invalid_request".to_string(),
                message: "the input must be sent as raw bytes".to_string(),
            });
        };
        let id = request
            .headers()
            .get(SESSION_HEADER)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| CommandError {
                code: "invalid_request".to_string(),
                message: "the session id header is missing".to_string(),
            })?;
        super::session_input(&state, id, data.clone())
    }

    #[tauri::command]
    pub fn session_resize(
        state: State<'_, AppState>,
        id: String,
        cols: u32,
        rows: u32,
    ) -> Result<(), CommandError> {
        super::session_resize(&state, &id, cols, rows)
    }

    #[tauri::command]
    pub fn session_ack(
        state: State<'_, AppState>,
        id: String,
        bytes: usize,
    ) -> Result<(), CommandError> {
        super::session_ack(&state, &id, bytes)
    }

    #[tauri::command]
    pub fn close_session(state: State<'_, AppState>, id: String) -> Result<(), CommandError> {
        super::close_session(&state, &id)
    }

    #[tauri::command]
    pub fn reconnect_session(
        state: State<'_, AppState>,
        id: String,
        cols: u32,
        rows: u32,
    ) -> Result<(), CommandError> {
        super::reconnect_session(&state, &id, cols, rows)
    }

    #[tauri::command]
    pub fn answer_prompt(
        state: State<'_, AppState>,
        id: String,
        prompt_id: u64,
        answer: PromptAnswer,
    ) -> Result<(), CommandError> {
        super::answer_prompt(&state, &id, prompt_id, answer)
    }

    #[tauri::command]
    pub fn resume_session(
        state: State<'_, AppState>,
        id: String,
        skip_credential: bool,
    ) -> Result<(), CommandError> {
        super::resume_session(&state, &id, skip_credential)
    }

    /// Opens a web link in the default browser; only `http` and `https`. Goes through the
    /// opener plugin's Rust API, so the webview needs no capability for it.
    #[tauri::command]
    pub fn open_external_url(app: AppHandle, url: String) -> Result<(), CommandError> {
        // Open the normalised URL that was validated, not the raw text.
        let parsed = crate::sessions::check_external_url(&url)?;
        app.opener()
            .open_url(parsed.as_str(), None::<&str>)
            .map_err(|_| CommandError {
                code: "open_failed".to_string(),
                message: "the link could not be opened".to_string(),
            })
    }

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
    use fernjo_core::{AppSettings, settings};
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
            close_on_exit: true,
        }
    }

    fn settings_path(config: &Path) -> PathBuf {
        config.join(settings::FILE_NAME)
    }

    #[test]
    fn unsaved_reason_distinguishes_env_unusable_and_no_config_dir() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Env);
        assert_eq!(unsaved_reason(&st), Some(UnsavedReason::Env));
        assert!(!persistent(&st));
        // Env wins over an unusable file.
        let mut both = state(data.path(), config.path(), DataDirSource::Env);
        both.settings_unusable = true;
        assert_eq!(unsaved_reason(&both), Some(UnsavedReason::Env));
        let mut bad = state(data.path(), config.path(), DataDirSource::Default);
        bad.settings_unusable = true;
        assert_eq!(unsaved_reason(&bad), Some(UnsavedReason::SettingsUnusable));
        let mut none = state(data.path(), config.path(), DataDirSource::Default);
        none.config_dir = None;
        assert_eq!(unsaved_reason(&none), Some(UnsavedReason::NoConfigDir));
        let info = get_terminal_settings(&bad).unwrap();
        assert_eq!(info.unsaved_reason, Some(UnsavedReason::SettingsUnusable));
        assert_eq!(
            serde_json::to_value(&info).unwrap()["unsavedReason"],
            "settingsUnusable"
        );
        assert_eq!(
            serde_json::to_value(UnsavedReason::NoConfigDir).unwrap(),
            "noConfigDir"
        );
    }

    #[test]
    fn unsaved_reason_is_none_when_saved() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        assert_eq!(unsaved_reason(&st), None);
        assert!(persistent(&st));
        let info = get_terminal_settings(&st).unwrap();
        assert_eq!(info.unsaved_reason, None);
        assert!(serde_json::to_value(&info).unwrap()["unsavedReason"].is_null());
    }

    #[test]
    fn close_on_exit_defaults_to_true_and_persists() {
        let [data, config] = dirs();
        let st = state(data.path(), config.path(), DataDirSource::Default);
        assert!(get_terminal_settings(&st).unwrap().close_on_exit);
        let mut off = input(14, 2000);
        off.close_on_exit = false;
        assert!(!set_terminal_settings(&st, off).unwrap().close_on_exit);
        assert!(!get_terminal_settings(&st).unwrap().close_on_exit);
        let saved = settings::load(&settings_path(config.path())).unwrap();
        assert_eq!(saved.terminal.unwrap().close_on_exit, Some(false));
        // An input without the key (an older UI) means true.
        let parsed: TerminalSettingsInput =
            serde_json::from_str(r#"{"fontSize":14,"scrollback":2000}"#).unwrap();
        assert!(parsed.close_on_exit);
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
                theme: None,
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
