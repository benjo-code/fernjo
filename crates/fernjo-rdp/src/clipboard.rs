//! Text clipboard rules: line endings and the size cap.

/// Largest clipboard text, in bytes, that goes in either direction.
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;

/// Local text to the form the remote side expects (CRLF line ends); `None` above the cap.
pub fn to_remote(text: &str) -> Option<String> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return None;
    }
    // CRLF first to LF, so text that already has CRLF is not doubled
    Some(text.replace("\r\n", "\n").replace('\n', "\r\n"))
}

/// Remote text to local form (LF line ends, no trailing NUL); `None` above the cap.
pub fn from_remote(text: &str) -> Option<String> {
    if text.len() > MAX_CLIPBOARD_BYTES {
        return None;
    }
    Some(text.trim_end_matches('\0').replace("\r\n", "\n"))
}

#[cfg(test)]
mod tests;
