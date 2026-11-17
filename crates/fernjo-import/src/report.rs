//! The import report: counts and three capped lists. Names, paths and protocols only, never a
//! password.

use serde::Serialize;

/// Every list is cut to this many lines; a final `and N more` line says how many were left out.
pub const MAX_LINES: usize = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub folders: usize,
    pub ssh: usize,
    pub rdp: usize,
    /// Vault entries the import creates.
    pub credentials: usize,
    /// Entries that more than one node references.
    pub shared_credentials: usize,
    /// Length of the full `skipped` list, before capping.
    pub skipped: usize,
    pub partial: usize,
    pub problems: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportLine {
    /// Where the node sits (`Folder / Sub / Name`); empty for aggregate lines.
    pub path: String,
    pub text: String,
}

/// Longest `path` or `text` of a line, in characters (an ellipsis counts).
pub const MAX_LINE_CHARS: usize = 200;

/// Cuts `s` to `MAX_LINE_CHARS` characters, ending in an ellipsis when it was longer.
fn clip(s: String) -> String {
    match s.char_indices().nth(MAX_LINE_CHARS) {
        None => s,
        Some(_) => {
            let end = s
                .char_indices()
                .nth(MAX_LINE_CHARS - 1)
                .map_or(s.len(), |(i, _)| i);
            format!("{}…", &s[..end])
        }
    }
}

impl ReportLine {
    /// A line; `path` and `text` are cut to `MAX_LINE_CHARS` characters.
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            path: clip(path.into()),
            text: clip(text.into()),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub counts: Counts,
    /// Nodes that were not imported at all.
    pub skipped: Vec<ReportLine>,
    /// Nodes that were imported with part of their settings left behind.
    pub partial: Vec<ReportLine>,
    /// Values that were malformed or missing.
    pub problems: Vec<ReportLine>,
}

/// Cuts `lines` to `MAX_LINES` and appends the `and N more` line.
pub(crate) fn cap(mut lines: Vec<ReportLine>) -> Vec<ReportLine> {
    if lines.len() > MAX_LINES {
        let more = lines.len() - MAX_LINES;
        lines.truncate(MAX_LINES);
        lines.push(ReportLine::new("", format!("and {more} more")));
    }
    lines
}
