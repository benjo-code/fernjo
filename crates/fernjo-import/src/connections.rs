//! fernjo's own connections file: `connections.json` plus a `format` marker and an export
//! time, without any credential reference (credentials are vault ids and mean nothing on
//! another machine).
//!
//! The file is the `connections.json` document, so `fernjo_core::persist::parse_document`
//! migrates and validates it unchanged and there is no second version number. The export
//! serialises a struct (not a `serde_json::Value`, which sorts keys) so the key order is
//! `format`, `exportedAt`, `schemaVersion`, `children`.

use fernjo_core::persist::{CURRENT_VERSION, parse_document};
use fernjo_core::{DocumentError, Node, NodeId, NodeKind, Protocol, Settings, Tree};
use serde::Serialize;
use serde_json::Value;

use crate::mremoteng::file::{MAX_DEPTH, MAX_FILE_BYTES, MAX_NODES};
use crate::naming::unique_name;
use crate::report::{Counts, Report, ReportLine};

/// Value of the `format` key.
pub const FORMAT: &str = "fernjo-connections";
const FOLDER_PREFIX: &str = "Imported from fernjo export";
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The exported file and how many credential references were left out of it.
#[derive(Debug)]
pub struct ConnectionsExport {
    pub bytes: Vec<u8>,
    pub dropped_credentials: usize,
}

/// What an import adds: one top-level folder holding the file's nodes, and the report.
#[derive(Debug)]
pub struct ConnectionsPlan {
    pub folder: Node,
    pub report: Report,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionsError {
    /// Not a fernjo connections export (a plain `connections.json`, a `confCons.xml`, a
    /// backup, random bytes).
    #[error("this is not a fernjo connections file")]
    NotThisFormat,
    #[error(
        "the file was written by a newer version of fernjo (file format {found}, this version understands {supported})"
    )]
    NewerSchema { found: u32, supported: u32 },
    #[error("the file is not usable: {0}")]
    Invalid(String),
}

impl ConnectionsError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotThisFormat => "not_this_format",
            Self::NewerSchema { .. } => "newer_schema",
            Self::Invalid(_) => "invalid",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileOut {
    format: &'static str,
    exported_at: String,
    schema_version: u32,
    children: Vec<Node>,
}

/// Clears every `settings.credential` below `nodes` and counts them.
fn strip_credentials(nodes: &mut [Node]) -> usize {
    let mut dropped = 0;
    for node in nodes {
        if node.settings.credential.take().is_some() {
            dropped += 1;
        }
        if let NodeKind::Folder { children } = &mut node.kind {
            dropped += strip_credentials(children);
        }
    }
    dropped
}

/// The export file for `tree`: pretty-printed JSON, no credential references.
/// `exported_at` is an ISO 8601 time chosen by the caller.
pub fn export(tree: &Tree, exported_at: &str) -> ConnectionsExport {
    let mut children = tree.children().to_vec();
    let dropped_credentials = strip_credentials(&mut children);
    let out = FileOut {
        format: FORMAT,
        exported_at: exported_at.to_owned(),
        schema_version: CURRENT_VERSION,
        children,
    };
    let bytes = serde_json::to_vec_pretty(&out).expect("a tree always serialises");
    ConnectionsExport {
        bytes,
        dropped_credentials,
    }
}

/// Gives every node a fresh id, drops credentials, counts what the report needs. `depth` is the
/// depth of `nodes` (top level of the file is 1); `total` counts nodes seen so far.
fn rebuild(
    nodes: &mut [Node],
    depth: usize,
    total: &mut usize,
    counts: &mut Counts,
) -> Result<usize, ConnectionsError> {
    let mut dropped = 0;
    for node in nodes {
        if depth > MAX_DEPTH {
            return Err(ConnectionsError::Invalid(format!(
                "the file is nested too deeply (at most {MAX_DEPTH} levels)"
            )));
        }
        *total += 1;
        if *total > MAX_NODES {
            return Err(ConnectionsError::Invalid(
                "the file holds too many connections and folders".to_owned(),
            ));
        }
        node.id = NodeId::new_v4();
        if node.settings.credential.take().is_some() {
            dropped += 1;
        }
        match &mut node.kind {
            NodeKind::Folder { children } => {
                counts.folders += 1;
                dropped += rebuild(children, depth + 1, total, counts)?;
            }
            NodeKind::Connection { protocol, .. } => match protocol {
                Protocol::Ssh => counts.ssh += 1,
                Protocol::Rdp => counts.rdp += 1,
            },
        }
    }
    Ok(dropped)
}

/// Reads an export file. `today` is `YYYY-MM-DD`; `taken_names` are the existing top-level names
/// (for the ` (2)`, ` (3)` suffix of the new folder).
///
/// The document goes through `parse_document`, so the core's migration and validation apply.
/// The core refuses a port of 0 for the whole file and silently renews repeated ids; it reports
/// nothing else about what it changed, so the report has no line for that. Ids are renewed here
/// anyway.
pub fn plan(
    bytes: &[u8],
    today: &str,
    taken_names: &[String],
) -> Result<ConnectionsPlan, ConnectionsError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ConnectionsError::Invalid(
            "the file is too large".to_owned(),
        ));
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        // serde_json refuses to nest deeper than 128 levels instead of overflowing the stack
        Err(e) if e.to_string().starts_with("recursion limit exceeded") => {
            return Err(ConnectionsError::Invalid(
                "the file is nested too deeply".to_owned(),
            ));
        }
        Err(_) => return Err(ConnectionsError::NotThisFormat),
    };
    if value.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ConnectionsError::NotThisFormat);
    }
    let tree = parse_document(value).map_err(|e| match e {
        DocumentError::NewerSchema { found, supported } => {
            ConnectionsError::NewerSchema { found, supported }
        }
        DocumentError::Invalid(m) => ConnectionsError::Invalid(m),
    })?;

    let mut children = tree.children().to_vec();
    let mut counts = Counts::default();
    let dropped = rebuild(&mut children, 1, &mut 0, &mut counts)?;

    let mut partial = Vec::new();
    if dropped > 0 {
        let s = if dropped == 1 { "" } else { "s" };
        partial.push(ReportLine::new(
            "",
            format!(
                "{dropped} credential reference{s} in the file {} not carried over (credentials stay in the vault)",
                if dropped == 1 { "was" } else { "were" }
            ),
        ));
    }
    counts.partial = partial.len();
    let name = unique_name(&format!("{FOLDER_PREFIX} ({today})"), taken_names);
    Ok(ConnectionsPlan {
        folder: Node {
            id: NodeId::new_v4(),
            name,
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Folder { children },
        },
        report: Report {
            counts,
            partial,
            ..Report::default()
        },
    })
}
