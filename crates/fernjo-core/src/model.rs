//! Data model: folders, connections, settings and the resolved (effective) view.

use serde::{Deserialize, Serialize};

/// Stable identity of a node (UUID v4).
pub type NodeId = uuid::Uuid;

/// Identity of a vault entry (UUID v4). The core only stores the reference; it never
/// knows the vault, so the id may dangle.
pub type CredentialId = uuid::Uuid;

/// Built-in default for the SSH keepalive interval in seconds (`0` = off).
pub const DEFAULT_KEEPALIVE_SECS: u32 = 30;
/// Built-in default port for SSH connections.
pub const DEFAULT_SSH_PORT: u16 = 22;
/// Built-in default port for RDP connections.
pub const DEFAULT_RDP_PORT: u16 = 3389;

/// Connection protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Rdp,
}

/// Inheritable settings. `None` means "inherit".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_secs: Option<u32>,
    /// Reference to a vault entry (schema 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialId>,
}

/// A node of the tree: a folder or a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub settings: Settings,
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// What a node is. Serialised as an internal `type` tag next to the node fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum NodeKind {
    Folder {
        #[serde(default)]
        children: Vec<Node>,
    },
    Connection {
        protocol: Protocol,
        #[serde(default)]
        host: String,
    },
}

/// Kind of a node without its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Folder,
    Connection,
}

/// Where an effective value comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Source {
    /// Set on the node itself.
    Local,
    /// Inherited from a folder above the node.
    Folder { id: NodeId, name: String },
    /// Built-in default (or no value at all).
    Default,
}

/// One setting as shown in the properties panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field<T> {
    pub local: Option<T>,
    pub effective: Option<T>,
    pub source: Source,
}

/// A node together with the effective value and source of each setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedNode {
    pub id: NodeId,
    pub kind: NodeType,
    pub name: String,
    pub description: String,
    pub protocol: Option<Protocol>,
    pub host: Option<String>,
    pub username: Field<String>,
    pub port: Field<u16>,
    pub keepalive_secs: Field<u32>,
    pub credential: Field<CredentialId>,
}
