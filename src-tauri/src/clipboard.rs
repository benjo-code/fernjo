//! Native clipboard read for the terminal's menu Paste.
//!
//! WebView2 asks for a permission before `navigator.clipboard.readText` answers, so the menu
//! Paste reads the OS clipboard here instead. The command returns the clipboard text by design
//! (like `reveal_credential`); nothing here ever puts clipboard content into an error, a log or
//! a `Debug` output.

use crate::commands::CommandError;

/// Largest text a paste returns. A bigger clipboard is refused, not cut: half a paste into a
/// shell is worse than none.
pub const MAX_PASTE_BYTES: usize = 1024 * 1024;

/// A source of clipboard text. The error is a fixed, content-free reason; the caller does not
/// show it.
pub trait ClipboardText {
    fn read_text(&self) -> Result<String, String>;
}

/// The operating system clipboard (text only).
pub struct OsClipboard;

impl ClipboardText for OsClipboard {
    fn read_text(&self) -> Result<String, String> {
        // The underlying error is dropped on purpose: only a fixed text leaves this function.
        let mut clipboard = arboard::Clipboard::new().map_err(|_| "unavailable".to_string())?;
        clipboard
            .get_text()
            .map_err(|_| "no text on the clipboard".to_string())
    }
}

fn error(code: &str, message: &str) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

/// Reads the clipboard text, refusing more than `MAX_PASTE_BYTES`. Both errors carry fixed
/// text only.
pub fn clipboard_read_text(clip: &dyn ClipboardText) -> Result<String, CommandError> {
    let text = clip
        .read_text()
        .map_err(|_| error("clipboard_unavailable", "The clipboard could not be read."))?;
    if text.len() > MAX_PASTE_BYTES {
        return Err(error(
            "clipboard_too_large",
            "The clipboard text is too large to paste.",
        ));
    }
    Ok(text)
}

pub mod handlers {
    use crate::commands::CommandError;

    /// A plain `async fn` command runs off the main thread; the read itself is a short blocking
    /// call, so it goes through `spawn_blocking`.
    #[tauri::command]
    pub async fn clipboard_read_text() -> Result<String, CommandError> {
        tauri::async_runtime::spawn_blocking(|| super::clipboard_read_text(&super::OsClipboard))
            .await
            .map_err(|_| CommandError::internal())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake(Result<String, String>);
    impl ClipboardText for Fake {
        fn read_text(&self) -> Result<String, String> {
            self.0.clone()
        }
    }

    #[test]
    fn read_returns_the_clipboard_text() {
        let got = clipboard_read_text(&Fake(Ok("ls -la\nçé".into()))).unwrap();
        assert_eq!(got, "ls -la\nçé");
        let exact = "a".repeat(MAX_PASTE_BYTES);
        assert_eq!(
            clipboard_read_text(&Fake(Ok(exact.clone()))).unwrap(),
            exact
        );
    }

    #[test]
    fn text_over_the_cap_is_refused_without_echoing_it() {
        let secret = format!("SENTINEL-{}", "a".repeat(MAX_PASTE_BYTES));
        let err = clipboard_read_text(&Fake(Ok(secret))).unwrap_err();
        assert_eq!(err.code, "clipboard_too_large");
        assert!(!err.message.contains("SENTINEL"));
        assert!(!format!("{err:?}").contains("SENTINEL"));
    }

    #[test]
    fn an_unavailable_clipboard_is_clipboard_unavailable() {
        let err = clipboard_read_text(&Fake(Err("no display".into()))).unwrap_err();
        assert_eq!(err.code, "clipboard_unavailable");
    }

    #[test]
    fn the_error_text_contains_no_clipboard_content() {
        // a misbehaving source that puts content into its error: it must not come through
        let err = clipboard_read_text(&Fake(Err("SENTINEL-content".into()))).unwrap_err();
        assert!(!err.message.contains("SENTINEL"));
        assert!(!format!("{err:?}").contains("SENTINEL"));
    }
}
