//! The snapshot: everything that is synced, as one JSON document (the plaintext of an envelope).
//! `connections` is the `connections.json` document, `vault` the `vault.fernjo` bytes (base64).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fernjo_core::json;
use fernjo_core::persist::{document, parse_document_bytes};
use fernjo_core::{DocumentError, MAX_TREE_DEPTH, Tree};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use uuid::Uuid;

use crate::envelope::GroupId;
use crate::error::{SyncError, snapshot_invalid};

/// The snapshot format version this build writes and understands.
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;

/// The device that wrote a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub format_version: u32,
    pub group_id: GroupId,
    pub seq: u64,
    pub writer: DeviceInfo,
    /// A label for the write, informational only: the engine writes the writer's local calendar
    /// date (`YYYY-MM-DD`), not a timestamp. No rule reads it and a reader must not parse it.
    pub written_at: String,
    pub connections: Tree,
    /// The `vault.fernjo` bytes exactly as on disk, if the writer has a vault.
    pub vault: Option<Vec<u8>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireOut<'a> {
    format_version: u32,
    group_id: GroupId,
    seq: u64,
    writer: &'a DeviceInfo,
    written_at: &'a str,
    connections: serde_json::Value,
    vault: Option<String>,
}

/// The snapshot as it is read: `connections` stays the borrowed JSON text until
/// `parse_document_bytes` reads it, so no part is built as a `serde_json::Value` (M11 review #28).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireIn<'a> {
    format_version: u32,
    group_id: GroupId,
    seq: u64,
    writer: DeviceInfo,
    written_at: String,
    #[serde(borrow)]
    connections: &'a RawValue,
    #[serde(default)]
    vault: Option<String>,
}

fn invalid(reason: impl std::fmt::Display) -> SyncError {
    snapshot_invalid(reason)
}

impl Snapshot {
    /// The JSON bytes to seal.
    pub fn to_plaintext(&self) -> Result<Vec<u8>, SyncError> {
        let wire = WireOut {
            format_version: self.format_version,
            group_id: self.group_id,
            seq: self.seq,
            writer: &self.writer,
            written_at: &self.written_at,
            connections: document(&self.connections),
            vault: self.vault.as_ref().map(|v| STANDARD.encode(v)),
        };
        serde_json::to_vec(&wire).map_err(invalid)
    }

    /// Parses a decrypted snapshot. A newer `formatVersion` is `newer_format`; a tree deeper
    /// than `MAX_TREE_DEPTH`, a bad document or bad base64 is `snapshot_invalid`.
    pub fn from_plaintext(bytes: &[u8]) -> Result<Snapshot, SyncError> {
        // validated and `formatVersion` read without building a `serde_json::Value`
        let [version] = json::head(bytes, ["formatVersion"]).map_err(invalid)?;
        let version = version
            .as_u64()
            .ok_or_else(|| invalid("missing formatVersion"))?;
        let version = u32::try_from(version).unwrap_or(u32::MAX);
        if version > SNAPSHOT_FORMAT_VERSION {
            return Err(SyncError::NewerFormat {
                found: version,
                supported: SNAPSHOT_FORMAT_VERSION,
            });
        }
        let wire: WireIn<'_> = serde_json::from_slice(bytes).map_err(invalid)?;
        let connections =
            parse_document_bytes(wire.connections.get().as_bytes()).map_err(|e| match e {
                DocumentError::NewerSchema { found, supported } => invalid(format!(
                    "connections schema {found} is newer than {supported}"
                )),
                DocumentError::Invalid(reason) => invalid(reason),
            })?;
        if connections.depth() > MAX_TREE_DEPTH {
            return Err(invalid("the tree is nested too deeply"));
        }
        let vault = wire
            .vault
            .map(|text| STANDARD.decode(text))
            .transpose()
            .map_err(|_| invalid("the vault is not valid base64"))?;
        Ok(Snapshot {
            format_version: wire.format_version,
            group_id: wire.group_id,
            seq: wire.seq,
            writer: wire.writer,
            written_at: wire.written_at,
            connections,
            vault,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fernjo_core::{Node, NodeKind, Protocol, Settings};

    fn connection(name: &str) -> Node {
        Node {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Connection {
                protocol: Protocol::Ssh,
                host: "h.example".to_string(),
            },
        }
    }

    fn folder(name: &str, children: Vec<Node>) -> Node {
        Node {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: String::new(),
            settings: Settings::default(),
            kind: NodeKind::Folder { children },
        }
    }

    fn snapshot(tree: Tree, vault: Option<Vec<u8>>) -> Snapshot {
        Snapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            group_id: Uuid::from_u128(9),
            seq: 3,
            writer: DeviceInfo {
                id: Uuid::from_u128(5),
                name: "laptop".to_string(),
            },
            written_at: "2026-11-22T10:00:00Z".to_string(),
            connections: tree,
            vault,
        }
    }

    fn as_value(snap: &Snapshot) -> serde_json::Value {
        serde_json::from_slice(&snap.to_plaintext().unwrap()).unwrap()
    }

    #[test]
    fn snapshot_round_trips_with_and_without_a_vault() {
        let tree = Tree::from_children(vec![folder("f", vec![connection("c")])]);
        for vault in [None, Some(vec![0u8, 1, 2, 255])] {
            let snap = snapshot(tree.clone(), vault);
            let back = Snapshot::from_plaintext(&snap.to_plaintext().unwrap()).unwrap();
            assert_eq!(back, snap);
        }
    }

    #[test]
    fn field_names_are_camel_case() {
        let text = String::from_utf8(snapshot(Tree::new(), None).to_plaintext().unwrap()).unwrap();
        for name in [
            "formatVersion",
            "groupId",
            "writtenAt",
            "connections",
            "writer",
        ] {
            assert!(text.contains(name), "{name} missing in {text}");
        }
    }

    fn nested(levels: usize) -> Tree {
        let mut node = connection("leaf");
        for _ in 1..levels {
            node = folder("f", vec![node]);
        }
        Tree::from_children(vec![node])
    }

    #[test]
    fn a_tree_deeper_than_60_levels_is_snapshot_invalid() {
        let ok = snapshot(nested(MAX_TREE_DEPTH), None);
        Snapshot::from_plaintext(&ok.to_plaintext().unwrap()).unwrap();
        let deep = snapshot(nested(MAX_TREE_DEPTH + 1), None);
        let err = Snapshot::from_plaintext(&deep.to_plaintext().unwrap()).unwrap_err();
        assert_eq!(err.code(), "snapshot_invalid");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let snap = snapshot(Tree::new(), None);
        let mut value = as_value(&snap);
        value["somethingNew"] = serde_json::json!({"a": 1});
        value["writer"]["extra"] = serde_json::json!(true);
        let back = Snapshot::from_plaintext(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn format_version_above_1_is_newer_format() {
        let mut value = as_value(&snapshot(Tree::new(), None));
        value["formatVersion"] = serde_json::json!(2);
        // a newer snapshot may have a shape this build cannot read at all
        value["connections"] = serde_json::json!("not a document");
        let err = Snapshot::from_plaintext(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(err.code(), "newer_format");
    }

    #[test]
    fn garbage_and_bad_base64_are_snapshot_invalid() {
        assert_eq!(
            Snapshot::from_plaintext(b"nope").unwrap_err().code(),
            "snapshot_invalid"
        );
        let mut value = as_value(&snapshot(Tree::new(), None));
        value["vault"] = serde_json::json!("!!!");
        let err = Snapshot::from_plaintext(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert_eq!(err.code(), "snapshot_invalid");
    }
    #[test]
    fn peer_text_in_a_snapshot_invalid_message_is_cut() {
        let big = "x".repeat(1024 * 1024);
        let tree = Tree::from_children(vec![connection("c")]);
        let mut bad_protocol = as_value(&snapshot(tree.clone(), None));
        bad_protocol["connections"]["children"][0]["protocol"] = serde_json::json!(big);
        let mut bad_port = as_value(&snapshot(tree, None));
        bad_port["connections"]["children"][0]["name"] = serde_json::json!(big);
        bad_port["connections"]["children"][0]["settings"]["port"] = serde_json::json!(0);
        let mut bad_type = as_value(&snapshot(Tree::new(), None));
        bad_type["seq"] = serde_json::json!(big);
        for value in [bad_protocol, bad_port, bad_type] {
            let err = Snapshot::from_plaintext(&serde_json::to_vec(&value).unwrap()).unwrap_err();
            assert_eq!(err.code(), "snapshot_invalid");
            let message = err.to_string();
            assert!(message.len() < 1024, "{} bytes", message.len());
            assert!(format!("{err:?}").len() < 1024);
        }
    }
}
