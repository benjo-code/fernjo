//! Data model: folders, connections, settings and the resolved (effective) view.

use std::fmt;

use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Unexpected, Visitor};
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
/// Built-in default colour depth (bits per pixel) of an RDP session.
pub const DEFAULT_RDP_COLOR_DEPTH: u8 = 32;
/// Colour depths an RDP session may ask for.
pub const RDP_COLOR_DEPTHS: [u8; 3] = [16, 24, 32];
/// Built-in default: the text clipboard is shared with the remote desktop.
pub const DEFAULT_RDP_CLIPBOARD: bool = true;
/// Smallest width or height of a fixed RDP resolution.
pub const MIN_RDP_DIMENSION: u16 = 200;
/// Largest width or height of a fixed RDP resolution (and of any remote desktop).
pub const MAX_RDP_DIMENSION: u16 = 8192;
/// Longest domain name setting, in characters.
pub const MAX_DOMAIN_CHARS: usize = 256;

/// Connection protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Ssh,
    Rdp,
}

/// How big the remote desktop of an RDP session is. `Deserialize` is written by hand (see
/// `crate::json`): the derived one for an internally tagged enum buffers every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum RdpResolution {
    /// The desktop follows the size of the tab.
    Fit,
    /// A fixed desktop size in pixels.
    Fixed { width: u16, height: u16 },
}

impl RdpResolution {
    /// A fixed size must lie between [`MIN_RDP_DIMENSION`] and [`MAX_RDP_DIMENSION`] on both sides.
    pub fn is_valid(&self) -> bool {
        match *self {
            RdpResolution::Fit => true,
            RdpResolution::Fixed { width, height } => {
                (MIN_RDP_DIMENSION..=MAX_RDP_DIMENSION).contains(&width)
                    && (MIN_RDP_DIMENSION..=MAX_RDP_DIMENSION).contains(&height)
            }
        }
    }
}

/// Whether `domain` is an acceptable domain setting: at most [`MAX_DOMAIN_CHARS`] characters and
/// no control characters.
pub fn is_valid_domain(domain: &str) -> bool {
    domain.chars().count() <= MAX_DOMAIN_CHARS && !domain.chars().any(char::is_control)
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
    /// Windows domain for RDP logins (schema 3). A domain on the vault entry wins over this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Desktop size of an RDP session (schema 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp_resolution: Option<RdpResolution>,
    /// Colour depth of an RDP session: 16, 24 or 32 (schema 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp_color_depth: Option<u8>,
    /// Whether an RDP session shares the text clipboard (schema 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rdp_clipboard: Option<bool>,
}

/// A node of the tree: a folder or a connection. `Deserialize` is written by hand (see
/// `crate::json`): the derived one for `#[serde(flatten)]` buffers every unknown field and the
/// whole `children` subtree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    pub domain: Field<String>,
    pub rdp_resolution: Field<RdpResolution>,
    pub rdp_color_depth: Field<u8>,
    pub rdp_clipboard: Field<bool>,
}

// ---- hand-written `Deserialize` for the file types --------------------------------------------
//
// They accept what the derived impls accepted for every document fernjo writes, read fields in
// any order, skip unknown fields with `IgnoredAny` and keep nothing they do not need, so the
// memory used is the size of the result (M11 review #28). A repeated key is refused, as the
// derived impls refuse it.

/// A field whose type depends on a tag that has not been seen yet: a string or an unsigned
/// integer is kept, anything else only as a description for the error message.
enum Deferred {
    Str(String),
    U64(u64),
    Bad(String),
}

impl Deferred {
    /// The error for this value where `expected` was wanted.
    fn invalid<E: de::Error>(&self, expected: &dyn de::Expected) -> E {
        match self {
            Deferred::Str(s) => E::invalid_type(Unexpected::Str(s), expected),
            Deferred::U64(v) => E::invalid_type(Unexpected::Unsigned(*v), expected),
            Deferred::Bad(what) => E::invalid_type(Unexpected::Other(what), expected),
        }
    }

    fn protocol<E: de::Error>(self) -> Result<Protocol, E> {
        match &self {
            Deferred::Str(s) if s == "ssh" => Ok(Protocol::Ssh),
            Deferred::Str(s) if s == "rdp" => Ok(Protocol::Rdp),
            Deferred::Str(s) => Err(E::unknown_variant(s, &["ssh", "rdp"])),
            _ => Err(self.invalid(&"variant identifier")),
        }
    }

    fn string<E: de::Error>(self) -> Result<String, E> {
        match self {
            Deferred::Str(s) => Ok(s),
            other => Err(other.invalid(&"a string")),
        }
    }

    fn u16<E: de::Error>(self) -> Result<u16, E> {
        match self {
            Deferred::U64(v) => {
                u16::try_from(v).map_err(|_| E::invalid_value(Unexpected::Unsigned(v), &"u16"))
            }
            other => Err(other.invalid(&"u16")),
        }
    }
}

/// `children` read before `type`: an array is read as nodes, anything else is kept only as a
/// description (a connection ignores it, a folder refuses it).
struct MaybeChildren(Result<Vec<Node>, Deferred>);

impl<'de> Deserialize<'de> for MaybeChildren {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(MaybeChildrenVisitor)
    }
}

struct MaybeChildrenVisitor;

impl<'de> Visitor<'de> for MaybeChildrenVisitor {
    type Value = MaybeChildren;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<MaybeChildren, A::Error> {
        let mut nodes = Vec::new();
        while let Some(node) = seq.next_element::<Node>()? {
            nodes.push(node);
        }
        Ok(MaybeChildren(Ok(nodes)))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_str(v).map(|d| MaybeChildren(Err(d)))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_u64(v).map(|d| MaybeChildren(Err(d)))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_i64(v).map(|d| MaybeChildren(Err(d)))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_f64(v).map(|d| MaybeChildren(Err(d)))
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_bool(v).map(|d| MaybeChildren(Err(d)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<MaybeChildren, E> {
        DeferredVisitor.visit_unit().map(|d| MaybeChildren(Err(d)))
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<MaybeChildren, A::Error> {
        DeferredVisitor
            .visit_map(map)
            .map(|d| MaybeChildren(Err(d)))
    }
}

impl<'de> Deserialize<'de> for Deferred {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(DeferredVisitor)
    }
}

struct DeferredVisitor;

impl<'de> Visitor<'de> for DeferredVisitor {
    type Value = Deferred;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Deferred, E> {
        Ok(Deferred::Str(v.to_owned()))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Deferred, E> {
        Ok(Deferred::U64(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Deferred, E> {
        Ok(Deferred::Bad(format!("integer `{v}`")))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Deferred, E> {
        Ok(Deferred::Bad(format!("floating point `{v}`")))
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Deferred, E> {
        Ok(Deferred::Bad(format!("boolean `{v}`")))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Deferred, E> {
        Ok(Deferred::Bad("null".to_owned()))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Deferred, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Deferred::Bad("sequence".to_owned()))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Deferred, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Deferred::Bad("map".to_owned()))
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(field_identifier, rename_all = "lowercase")]
enum NodeKey {
    Id,
    Name,
    Description,
    Settings,
    Type,
    Children,
    Protocol,
    Host,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum NodeTag {
    Folder,
    Connection,
}

/// The error for a key that appears twice in one object, as the derived impls give it.
fn refuse_repeated<E: de::Error>(repeated: bool, name: &'static str) -> Result<(), E> {
    if repeated {
        Err(E::duplicate_field(name))
    } else {
        Ok(())
    }
}

impl<'de> Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(NodeVisitor)
    }
}

struct NodeVisitor;

impl<'de> Visitor<'de> for NodeVisitor {
    type Value = Node;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a node object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
        let mut id = None;
        let mut name = None;
        let mut description = None;
        let mut settings = None;
        let mut tag = None;
        let mut children: Option<Result<Vec<Node>, Deferred>> = None;
        let mut protocol = None;
        let mut host = None;
        while let Some(key) = map.next_key::<NodeKey>()? {
            let (repeated, field) = match key {
                NodeKey::Id => (id.is_some(), "id"),
                NodeKey::Name => (name.is_some(), "name"),
                NodeKey::Description => (description.is_some(), "description"),
                NodeKey::Settings => (settings.is_some(), "settings"),
                NodeKey::Type => (tag.is_some(), "type"),
                NodeKey::Children => (children.is_some(), "children"),
                NodeKey::Protocol => (protocol.is_some(), "protocol"),
                NodeKey::Host => (host.is_some(), "host"),
                NodeKey::Other => (false, ""),
            };
            refuse_repeated(repeated, field)?;
            // the fields of the other kind are ignored, as the derived (tagged) impl did
            match (key, tag) {
                (NodeKey::Id, _) => id = Some(map.next_value::<NodeId>()?),
                (NodeKey::Name, _) => name = Some(map.next_value::<String>()?),
                (NodeKey::Description, _) => description = Some(map.next_value::<String>()?),
                (NodeKey::Settings, _) => settings = Some(map.next_value::<Settings>()?),
                (NodeKey::Type, _) => tag = Some(map.next_value::<NodeTag>()?),
                (NodeKey::Children, Some(NodeTag::Connection))
                | (NodeKey::Protocol | NodeKey::Host, Some(NodeTag::Folder))
                | (NodeKey::Other, _) => {
                    map.next_value::<IgnoredAny>()?;
                }
                (NodeKey::Children, Some(NodeTag::Folder)) => {
                    children = Some(Ok(map.next_value::<Vec<Node>>()?));
                }
                (NodeKey::Children, None) => children = Some(map.next_value::<MaybeChildren>()?.0),
                (NodeKey::Protocol, Some(NodeTag::Connection)) => {
                    protocol = Some(Ok(map.next_value::<Protocol>()?));
                }
                (NodeKey::Protocol, None) => protocol = Some(Err(map.next_value::<Deferred>()?)),
                (NodeKey::Host, Some(NodeTag::Connection)) => {
                    host = Some(Deferred::Str(map.next_value::<String>()?));
                }
                (NodeKey::Host, None) => host = Some(map.next_value::<Deferred>()?),
            }
        }
        let id = id.ok_or_else(|| de::Error::missing_field("id"))?;
        let name = name.ok_or_else(|| de::Error::missing_field("name"))?;
        let kind = match tag.ok_or_else(|| de::Error::missing_field("type"))? {
            NodeTag::Folder => NodeKind::Folder {
                children: match children {
                    None => Vec::new(),
                    Some(Ok(children)) => children,
                    Some(Err(other)) => return Err(other.invalid(&"a sequence")),
                },
            },
            NodeTag::Connection => NodeKind::Connection {
                protocol: match protocol.ok_or_else(|| de::Error::missing_field("protocol"))? {
                    Ok(p) => p,
                    Err(deferred) => deferred.protocol()?,
                },
                host: host.map(Deferred::string).transpose()?.unwrap_or_default(),
            },
        };
        Ok(Node {
            id,
            name,
            description: description.unwrap_or_default(),
            settings: settings.unwrap_or_default(),
            kind,
        })
    }
}

#[derive(Deserialize, Clone, Copy)]
#[serde(field_identifier, rename_all = "lowercase")]
enum ResolutionKey {
    Mode,
    Width,
    Height,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum ResolutionTag {
    Fit,
    Fixed,
}

impl<'de> Deserialize<'de> for RdpResolution {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(ResolutionVisitor)
    }
}

struct ResolutionVisitor;

impl<'de> Visitor<'de> for ResolutionVisitor {
    type Value = RdpResolution;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an RDP resolution object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RdpResolution, A::Error> {
        let mut mode = None;
        let mut width = None;
        let mut height = None;
        while let Some(key) = map.next_key::<ResolutionKey>()? {
            let (repeated, field) = match key {
                ResolutionKey::Mode => (mode.is_some(), "mode"),
                ResolutionKey::Width => (width.is_some(), "width"),
                ResolutionKey::Height => (height.is_some(), "height"),
                ResolutionKey::Other => (false, ""),
            };
            refuse_repeated(repeated, field)?;
            match (key, mode) {
                (ResolutionKey::Mode, _) => mode = Some(map.next_value::<ResolutionTag>()?),
                (ResolutionKey::Width, Some(ResolutionTag::Fixed)) => {
                    width = Some(Deferred::U64(map.next_value::<u16>()?.into()));
                }
                (ResolutionKey::Height, Some(ResolutionTag::Fixed)) => {
                    height = Some(Deferred::U64(map.next_value::<u16>()?.into()));
                }
                (ResolutionKey::Width, None) => width = Some(map.next_value::<Deferred>()?),
                (ResolutionKey::Height, None) => height = Some(map.next_value::<Deferred>()?),
                (ResolutionKey::Width | ResolutionKey::Height, Some(ResolutionTag::Fit))
                | (ResolutionKey::Other, _) => {
                    map.next_value::<IgnoredAny>()?;
                }
            }
        }
        match mode.ok_or_else(|| de::Error::missing_field("mode"))? {
            ResolutionTag::Fit => Ok(RdpResolution::Fit),
            ResolutionTag::Fixed => Ok(RdpResolution::Fixed {
                width: width
                    .ok_or_else(|| de::Error::missing_field("width"))?
                    .u16()?,
                height: height
                    .ok_or_else(|| de::Error::missing_field("height"))?
                    .u16()?,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "6f0c1c2a-0000-4000-8000-000000000001";

    #[test]
    fn a_node_reads_its_fields_in_any_order_and_ignores_the_other_kind() {
        let written = json!({"id": ID, "name": "n", "description": "d", "settings": {"port": 2},
            "type": "connection", "protocol": "rdp", "host": "h", "x": [1]});
        let reordered = json!({"host": "h", "protocol": "rdp", "x": {"a": 1}, "type": "connection",
            "settings": {"port": 2}, "description": "d", "name": "n", "id": ID, "children": 5});
        let a: Node = serde_json::from_value(written.clone()).unwrap();
        let b: Node = serde_json::from_str(&reordered.to_string()).unwrap();
        assert_eq!(a, b);
        let mut expected = written;
        expected.as_object_mut().unwrap().remove("x");
        assert_eq!(serde_json::to_value(&a).unwrap(), expected);
        // a folder ignores connection fields, also of the wrong type, before or after `type`
        for folder in [
            json!({"id": ID, "name": "f", "type": "folder", "protocol": 7, "host": []}),
            json!({"protocol": "telnet", "host": null, "id": ID, "name": "f", "type": "folder"}),
        ] {
            let node: Node = serde_json::from_str(&folder.to_string()).unwrap();
            assert_eq!(node.kind, NodeKind::Folder { children: vec![] });
        }
    }

    #[test]
    fn a_node_refuses_what_the_derived_impl_refused() {
        let connection = |extra: serde_json::Value| {
            let mut v = json!({"id": ID, "name": "n", "type": "connection"});
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        for (bad, expected) in [
            (json!({"name": "n", "type": "folder"}), "missing field `id`"),
            (json!({"id": ID, "type": "folder"}), "missing field `name`"),
            (json!({"id": ID, "name": "n"}), "missing field `type`"),
            (
                json!({"id": ID, "name": "n", "type": "x"}),
                "unknown variant `x`",
            ),
            (connection(json!({})), "missing field `protocol`"),
            (connection(json!({"protocol": "x"})), "unknown variant `x`"),
            (
                json!({"protocol": "x", "id": ID, "name": "n", "type": "connection"}),
                "unknown variant `x`",
            ),
            (
                json!({"host": 5, "protocol": "ssh", "id": ID, "name": "n", "type": "connection"}),
                "invalid type",
            ),
            (
                connection(json!({"protocol": "ssh", "host": 5})),
                "invalid type",
            ),
            (
                json!({"id": ID, "name": "n", "type": "folder", "children": {}}),
                "invalid type",
            ),
            (
                json!({"id": ID, "name": "n", "type": "folder", "description": null}),
                "invalid type",
            ),
            (json!(["x"]), "invalid type"),
        ] {
            let err = serde_json::from_str::<Node>(&bad.to_string())
                .unwrap_err()
                .to_string();
            assert!(err.contains(expected), "{bad}: {err}");
        }
    }

    #[test]
    fn a_repeated_key_is_refused_like_the_derived_impl_refuses_it() {
        for (text, field) in [
            (
                format!(r#"{{"id":"{ID}","name":"a","name":"b","type":"folder"}}"#),
                "name",
            ),
            (
                format!(r#"{{"type":"folder","id":"{ID}","name":"a","type":"connection"}}"#),
                "type",
            ),
            (r#"{"mode":"fit","mode":"fixed"}"#.to_owned(), "mode"),
        ] {
            let err = if field == "mode" {
                serde_json::from_str::<RdpResolution>(&text).unwrap_err()
            } else {
                serde_json::from_str::<Node>(&text).unwrap_err()
            };
            assert!(
                err.to_string()
                    .contains(&format!("duplicate field `{field}`")),
                "{text}: {err}"
            );
        }
    }

    #[test]
    fn an_rdp_resolution_reads_like_the_derived_impl() {
        let fixed = RdpResolution::Fixed {
            width: 800,
            height: 600,
        };
        for (text, expected) in [
            (r#"{"mode":"fit"}"#, RdpResolution::Fit),
            (r#"{"mode":"fit","width":"x","y":[1]}"#, RdpResolution::Fit),
            (r#"{"width":-1,"mode":"fit"}"#, RdpResolution::Fit),
            (r#"{"mode":"fixed","width":800,"height":600,"y":{}}"#, fixed),
            (r#"{"height":600,"width":800,"mode":"fixed"}"#, fixed),
        ] {
            let got: RdpResolution = serde_json::from_str(text).unwrap();
            assert_eq!(got, expected, "{text}");
        }
        for text in [
            r#"{}"#,
            r#"{"mode":"huge"}"#,
            r#"{"mode":"fixed","width":800}"#,
            r#"{"mode":"fixed","width":70000,"height":600}"#,
            r#"{"width":70000,"height":600,"mode":"fixed"}"#,
            r#"{"width":"800","height":600,"mode":"fixed"}"#,
            r#"{"mode":"fixed","width":-1,"height":600}"#,
        ] {
            assert!(
                serde_json::from_str::<RdpResolution>(text).is_err(),
                "{text}"
            );
        }
        for r in [RdpResolution::Fit, fixed] {
            let back: RdpResolution =
                serde_json::from_value(serde_json::to_value(r).unwrap()).unwrap();
            assert_eq!(back, r);
        }
    }
}
