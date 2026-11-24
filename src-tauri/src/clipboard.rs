//! Native clipboard read for the terminal's menu Paste.
//!
//! WebView2 asks for a permission before `navigator.clipboard.readText` answers, so the menu
//! Paste reads the OS clipboard here instead. The command returns the clipboard text by design
//! (like `reveal_credential`); nothing here ever puts clipboard content into an error, a log or
//! a `Debug` output.

use std::sync::Arc;
use std::time::Duration;

use crate::commands::CommandError;

/// How long a paste waits for the OS clipboard. A clipboard owner that never answers (a hung
/// application, a stuck Wayland data source) must not hang the paste.
pub const CLIPBOARD_READ_LIMIT: Duration = Duration::from_secs(2);

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

/// Runs the blocking read off the async executor and gives up after `limit`: the answer is the
/// fixed `clipboard_unavailable` error, never content. A read that really hangs keeps its
/// blocking thread until the OS call returns (bounded per paste, nothing waits for it).
pub async fn read_with_limit(
    clip: Arc<dyn ClipboardText + Send + Sync>,
    limit: Duration,
) -> Result<String, CommandError> {
    let unavailable = || error("clipboard_unavailable", "The clipboard could not be read.");
    let read = tokio::task::spawn_blocking(move || clipboard_read_text(&*clip));
    match tokio::time::timeout(limit, read).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) | Err(_) => Err(unavailable()),
    }
}

pub mod handlers {
    use crate::commands::CommandError;

    /// A plain `async fn` command runs off the main thread; the read is a blocking call with a
    /// time limit (`read_with_limit`).
    #[tauri::command]
    pub async fn clipboard_read_text() -> Result<String, CommandError> {
        super::read_with_limit(
            std::sync::Arc::new(super::OsClipboard),
            super::CLIPBOARD_READ_LIMIT,
        )
        .await
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

    struct Hanging;
    impl ClipboardText for Hanging {
        fn read_text(&self) -> Result<String, String> {
            std::thread::sleep(Duration::from_millis(600));
            Ok("SENTINEL-late".into())
        }
    }

    #[tokio::test]
    async fn a_clipboard_read_that_hangs_ends_with_clipboard_unavailable() {
        let err = read_with_limit(Arc::new(Hanging), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(err.code, "clipboard_unavailable");
        assert!(!err.message.contains("SENTINEL"));
        assert!(!format!("{err:?}").contains("SENTINEL"));
    }

    #[tokio::test]
    async fn a_prompt_read_passes_through_read_with_limit() {
        let got = read_with_limit(Arc::new(Fake(Ok("hi".into()))), CLIPBOARD_READ_LIMIT)
            .await
            .unwrap();
        assert_eq!(got, "hi");
    }

    #[test]
    fn the_error_text_contains_no_clipboard_content() {
        // a misbehaving source that puts content into its error: it must not come through
        let err = clipboard_read_text(&Fake(Err("SENTINEL-content".into()))).unwrap_err();
        assert!(!err.message.contains("SENTINEL"));
        assert!(!format!("{err:?}").contains("SENTINEL"));
    }
}
