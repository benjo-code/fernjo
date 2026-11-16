//! Reading a mRemoteNG `confCons.xml` (ConfVersion 2.6) into a raw, unmapped document.
//!
//! Order of checks: size, UTF-8 and BOM, XML parse (no DTD), root element and version, encryption
//! engine, `KdfIterations` range, key check through `Protected`, optional full-file decryption, then a
//! depth-limited walk with a node cap. Error messages never contain secrets.

use std::fmt;

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::crypto::{self, CryptoError, DEFAULT_KEY, MAX_ITERATIONS, MIN_ITERATIONS};
use crate::error::ImportError;

/// Files above this size are refused before parsing.
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum container nesting (the root's children are level 1).
pub const MAX_DEPTH: usize = 64;
/// Maximum number of `Node` elements.
pub const MAX_NODES: usize = 100_000;
/// mRemoteNG's own default when `KdfIterations` is absent.
const DEFAULT_ITERATIONS: u32 = 1000;
/// The plaintext of the `Protected` attribute when the key is right.
const PROTECTED_PLAINTEXT: &str = "ThisIsNotProtected";
/// Parser guard on all DOM nodes (elements and text); the real cap is `MAX_NODES` elements.
const XML_NODES_LIMIT: u32 = 2_000_000;
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawKind {
    Container,
    Connection,
}

/// Which settings the node takes from its parent (`Inherit*` attributes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inherit {
    pub username: bool,
    pub password: bool,
    pub domain: bool,
    pub port: bool,
}

#[derive(Clone)]
pub struct RawNode {
    pub kind: RawKind,
    pub name: String,
    pub description: String,
    pub hostname: String,
    pub protocol: String,
    pub port: Option<u32>,
    pub username: String,
    pub domain: String,
    pub password: Option<Zeroizing<String>>,
    /// The `Password` attribute was present but could not be decrypted.
    pub password_error: bool,
    pub inherit: Inherit,
    pub putty_session: String,
    pub children: Vec<RawNode>,
}

impl fmt::Debug for RawNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RawNode")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("hostname", &self.hostname)
            .field("protocol", &self.protocol)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("has_password", &self.password.is_some())
            .field("password_error", &self.password_error)
            .field("inherit", &self.inherit)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct RawDocument {
    pub nodes: Vec<RawNode>,
}

fn unsupported(msg: impl Into<String>) -> ImportError {
    ImportError::UnsupportedFormat(msg.into())
}

/// Shortens an attribute value for use in a message.
fn shown(s: &str) -> String {
    let mut out: String = s.chars().take(32).collect();
    if s.chars().count() > 32 {
        out.push('…');
    }
    out
}

fn parse(text: &str) -> Result<roxmltree::Document<'_>, ImportError> {
    let opts = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: XML_NODES_LIMIT,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, opts).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => ImportError::TooComplex,
        _ => unsupported(
            "the file is not a readable XML document (pre-2.6 mRemoteNG files and other formats are not supported)",
        ),
    })
}

fn flag(node: roxmltree::Node, name: &str) -> bool {
    node.attribute(name)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

fn attr(node: roxmltree::Node, name: &str) -> String {
    node.attribute(name).unwrap_or_default().to_owned()
}

fn malformed(e: CryptoError) -> ImportError {
    match e {
        CryptoError::BadBase64 | CryptoError::TooShort => {
            ImportError::Damaged("an encrypted value is malformed".to_owned())
        }
        CryptoError::Authentication => {
            ImportError::Damaged("an encrypted value cannot be decrypted".to_owned())
        }
    }
}

struct Ctx<'a> {
    key: &'a SecretString,
    iterations: u32,
}

/// Reads a mRemoteNG 2.6 file. `key` is the file's password; without one the default key is tried.
pub fn read(bytes: &[u8], key: Option<&SecretString>) -> Result<RawDocument, ImportError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes)
        .map_err(|_| unsupported("the file is not UTF-8 text (it may be binary or encrypted)"))?;
    let xml = parse(text)?;
    let root = xml.root_element();

    if root.tag_name().name() != "Connections" {
        return Err(unsupported(format!(
            "the XML root is '{}', not a mRemoteNG 'Connections' document",
            shown(root.tag_name().name())
        )));
    }
    let version = root.attribute("ConfVersion").ok_or_else(|| {
        unsupported("the file has no ConfVersion (only mRemoteNG 2.6 files are supported)")
    })?;
    if version.trim() != "2.6" {
        return Err(unsupported(format!(
            "ConfVersion {} is not supported, only 2.6 (older files and the 2.7/2.8 pre-release files are on the backlog)",
            shown(version)
        )));
    }
    let engine = root.attribute("EncryptionEngine").unwrap_or_default();
    if !engine.trim().eq_ignore_ascii_case("AES") {
        return Err(unsupported(format!(
            "EncryptionEngine '{}' is not supported, only AES",
            shown(engine)
        )));
    }
    let mode = root.attribute("BlockCipherMode").unwrap_or_default();
    if !mode.trim().eq_ignore_ascii_case("GCM") {
        return Err(unsupported(format!(
            "BlockCipherMode '{}' is not supported, only GCM",
            shown(mode)
        )));
    }

    // Range check here: `decrypt` reports out-of-range iterations as an authentication failure,
    // which would read as a wrong key.
    let iterations = match root.attribute("KdfIterations") {
        None => DEFAULT_ITERATIONS,
        Some(v) => match v.trim().parse::<u32>() {
            Ok(n) if (MIN_ITERATIONS..=MAX_ITERATIONS).contains(&n) => n,
            _ => {
                return Err(ImportError::Damaged(format!(
                    "KdfIterations '{}' is not a number between {MIN_ITERATIONS} and {MAX_ITERATIONS}",
                    shown(v)
                )));
            }
        },
    };

    let protected = root
        .attribute("Protected")
        .ok_or_else(|| ImportError::Damaged("the file has no Protected attribute".to_owned()))?;
    let default_key = SecretString::from(DEFAULT_KEY.to_owned());
    let (used_key, failure) = match key {
        Some(k) => (k, ImportError::WrongKey),
        None => (&default_key, ImportError::KeyNeeded),
    };
    match crypto::decrypt(protected, used_key, iterations) {
        Ok(p) if p.as_str() == PROTECTED_PLAINTEXT => {}
        Ok(_) | Err(CryptoError::Authentication) => return Err(failure),
        Err(e) => return Err(malformed(e)),
    }

    let ctx = Ctx {
        key: used_key,
        iterations,
    };
    let mut counter = 0usize;
    if !flag(root, "FullFileEncryption") {
        let nodes = walk(root, 1, &mut counter, &ctx)?;
        return Ok(RawDocument { nodes });
    }

    let cipher: String = root
        .children()
        .filter(|c| c.is_text())
        .filter_map(|c| c.text())
        .collect();
    let inner = crypto::decrypt(cipher.trim(), used_key, iterations).map_err(malformed)?;
    if inner.len() > MAX_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let wrapped = Zeroizing::new(format!("<Connections>{}</Connections>", inner.as_str()));
    let inner_xml = parse(&wrapped).map_err(|e| match e {
        ImportError::UnsupportedFormat(_) => {
            ImportError::Damaged("the decrypted file body is not valid XML".to_owned())
        }
        other => other,
    })?;
    let nodes = walk(inner_xml.root_element(), 1, &mut counter, &ctx)?;
    Ok(RawDocument { nodes })
}

/// Reads the `Node` children of `parent`. `depth` is the level of those children (the root's
/// children are level 1); it is checked before descending, so recursion never exceeds `MAX_DEPTH`.
fn walk(
    parent: roxmltree::Node,
    depth: usize,
    counter: &mut usize,
    ctx: &Ctx,
) -> Result<Vec<RawNode>, ImportError> {
    let mut out = Vec::new();
    for el in parent
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "Node")
    {
        *counter += 1;
        if depth > MAX_DEPTH || *counter > MAX_NODES {
            return Err(ImportError::TooComplex);
        }
        out.push(read_node(el, depth, counter, ctx)?);
    }
    Ok(out)
}

fn read_node(
    el: roxmltree::Node,
    depth: usize,
    counter: &mut usize,
    ctx: &Ctx,
) -> Result<RawNode, ImportError> {
    let name = attr(el, "Name");
    let kind = match el.attribute("Type").map(str::trim) {
        Some("Container") => RawKind::Container,
        Some("Connection") | None => RawKind::Connection,
        Some(other) => {
            return Err(ImportError::Damaged(format!(
                "node '{}' has the unknown type '{}'",
                shown(&name),
                shown(other)
            )));
        }
    };
    let port = match el.attribute("Port").map(str::trim) {
        None | Some("") => None,
        Some(p) => Some(p.parse::<u32>().map_err(|_| {
            ImportError::Damaged(format!(
                "node '{}' has the invalid port '{}'",
                shown(&name),
                shown(p)
            ))
        })?),
    };
    let (password, password_error) = match el.attribute("Password").map(str::trim) {
        None | Some("") => (None, false),
        Some(secret) => match crypto::decrypt(secret, ctx.key, ctx.iterations) {
            Ok(p) if p.is_empty() => (None, false),
            Ok(p) => (Some(p), false),
            Err(_) => (None, true),
        },
    };
    let children = match kind {
        RawKind::Container => walk(el, depth + 1, counter, ctx)?,
        RawKind::Connection => Vec::new(),
    };
    Ok(RawNode {
        kind,
        description: attr(el, "Descr"),
        hostname: attr(el, "Hostname"),
        protocol: attr(el, "Protocol"),
        port,
        username: attr(el, "Username"),
        domain: attr(el, "Domain"),
        password,
        password_error,
        inherit: Inherit {
            username: flag(el, "InheritUsername"),
            password: flag(el, "InheritPassword"),
            domain: flag(el, "InheritDomain"),
            port: flag(el, "InheritPort"),
        },
        putty_session: attr(el, "PuttySession"),
        name,
        children,
    })
}
