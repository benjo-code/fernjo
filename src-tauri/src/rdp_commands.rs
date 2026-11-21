//! Tauri commands that only exist for RDP sessions: keyboard and mouse input and the text
//! clipboard. Opening, resizing, acknowledging, closing, reconnecting and answering prompts reuse
//! the commands in `session_commands.rs` (for an RDP session the sizes are pixels).
//!
//! Like `commands.rs`: plain functions over `&AppState` plus thin wrappers in [`handlers`].

use fernjo_rdp::InputEvent;

use crate::commands::CommandError;
use crate::state::AppState;

/// Sends keyboard and mouse events to a session (desktop pixel coordinates, clamped to the
/// desktop by the session).
///
/// # Errors
/// `not_found` for an unknown id, `session_not_connected` without a live session,
/// `too_many_events` above 256 events in one call.
pub fn rdp_input(state: &AppState, id: &str, events: Vec<InputEvent>) -> Result<(), CommandError> {
    state.sessions.rdp_input(id, events)
}

/// Offers text copied in the UI to the remote clipboard.
///
/// # Errors
/// `not_found` for an unknown id, `session_not_connected` without a live session.
pub fn rdp_clipboard(state: &AppState, id: &str, text: String) -> Result<(), CommandError> {
    state.sessions.rdp_clipboard(id, text)
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::State;

    #[tauri::command]
    pub fn rdp_input(
        state: State<'_, AppState>,
        id: String,
        events: Vec<InputEvent>,
    ) -> Result<(), CommandError> {
        super::rdp_input(&state, &id, events)
    }

    #[tauri::command]
    pub fn rdp_clipboard(
        state: State<'_, AppState>,
        id: String,
        text: String,
    ) -> Result<(), CommandError> {
        super::rdp_clipboard(&state, &id, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sessions_are_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::open(Some(dir.path().to_path_buf()));
        assert_eq!(
            rdp_input(&st, "nope", Vec::new()).unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            rdp_clipboard(&st, "nope", "x".into()).unwrap_err().code,
            "not_found"
        );
    }

    #[test]
    fn input_events_deserialise_from_the_wire_shapes() {
        let events: Vec<InputEvent> = serde_json::from_str(
            r#"[{"kind":"key","scancode":30,"extended":false,"pressed":true},
                {"kind":"mouseMove","x":10,"y":20},
                {"kind":"mouseButton","button":"left","pressed":true},
                {"kind":"wheel","vertical":true,"delta":-120}]"#,
        )
        .unwrap();
        assert_eq!(events.len(), 4);
        // an unknown kind is refused, not guessed
        assert!(serde_json::from_str::<Vec<InputEvent>>(r#"[{"kind":"nuke"}]"#).is_err());
    }
}
