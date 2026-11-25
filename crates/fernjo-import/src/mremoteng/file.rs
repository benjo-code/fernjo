//! Reading a mRemoteNG `confCons.xml` (ConfVersion 2.6) into a raw, unmapped document.
//!
//! Order of checks: size, UTF-8 and BOM, a byte scan for attribute and namespace floods, XML parse
//! (no DTD), root element and version, encryption engine, `KdfIterations` range, the work budget (stored passwords x iterations), key check through
//! `Protected`, optional full-file decryption (and the budget again for the decrypted body), then a
//! depth-limited walk with a node cap. Error messages never contain secrets.

use std::fmt;

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::crypto::{self, CryptoError, DEFAULT_KEY, MAX_ITERATIONS, MIN_ITERATIONS};
use super::scan;
use crate::error::ImportError;

/// Files above this size are refused before parsing.
pub const MAX_FILE_BYTES: usize = 32 * 1024 * 1024;
/// Maximum container nesting (the root's children are level 1). One less than the core's
/// `MAX_TREE_DEPTH`, because the plan puts everything below one new top-level folder.
pub const MAX_DEPTH: usize = fernjo_core::MAX_TREE_DEPTH - 1;
/// Maximum number of `Node` elements.
pub const MAX_NODES: usize = 100_000;
/// Most PBKDF2 iterations one file may cost in all: the key check, a full-file body, and one
/// derivation per stored `Password` (each has its own salt). The normal case is hundreds of
/// passwords at 1000 iterations (well below a million); the cap keeps a hostile file from
/// occupying the importer for hours (it is checked before any password is decrypted).
pub const MAX_PBKDF2_ITERATIONS: u64 = 20_000_000;
/// mRemoteNG's own default when `KdfIterations` is absent.
const DEFAULT_ITERATIONS: u32 = 1000;
/// The plaintext of the `Protected` attribute when the key is right.
const PROTECTED_PLAINTEXT: &str = "ThisIsNotProtected";
/// Most attributes one start tag may carry (a node of a real 2.6 file, the owner's fixture
/// `owner-confCons.xml`, carries 114, so the cap is about 1.75 times that). `roxmltree` checks
/// every new attribute against the earlier ones, so one tag with 100 000 attributes costs 21 s.
pub const MAX_ATTRIBUTES_PER_TAG: usize = 200;
/// Most namespace declarations (`xmlns`, `xmlns:p`) a document may carry in all (a real file has one,
/// `xmlns:mrng` on the root). Each declaration is searched again by every later prefixed name, which
/// is cubic over nested elements: 5 000 of them cost 55 s.
pub const MAX_XMLNS_DECLARATIONS: usize = 8;
/// Longest `Name` in characters. Longer names are cut (and reported): a name is copied into report
/// lines, trails and the tree, so its length must not be attacker-chosen.
pub const MAX_NAME_CHARS: usize = 256;
/// Longest `Hostname` (a DNS name has at most 253). A longer value cannot be a host, so it is
/// dropped (empty host, reported), never cut into a different host.
pub const MAX_HOST_CHARS: usize = 256;
/// Longest `Username`, `Domain` and `PuttySession` (cut; username and domain are reported, the
/// PuTTY session is only shown in a report line).
pub const MAX_IDENTITY_CHARS: usize = 256;
/// Longest `Descr`; longer text is cut and reported.
pub const MAX_DESCRIPTION_CHARS: usize = 4096;
/// Longest `Protocol`; the only values that matter are `SSH2` and `RDP`.
pub const MAX_PROTOCOL_CHARS: usize = 64;
/// Longest `Password` attribute (base64 of salt, nonce, ciphertext and tag; a few hundred
/// characters in practice). A longer one is treated as not decryptable, without deriving a key.
pub const MAX_PASSWORD_CHARS: usize = 16 * 1024;
/// Parser guard on all DOM nodes (elements and text); the real cap is `MAX_NODES` elements.
const XML_NODES_LIMIT: u32 = 2_000_000;
const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawKind {
    Container,
    Connection,
    /// A `Type` that is neither `Container` nor `Connection` (the raw text, shortened). The node and
    /// its children are read; the mapping decides what to do with them.
    Unknown(String),
}

/// Which settings the node takes from its parent (`Inherit*` attributes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Inherit {
    pub username: bool,
    pub password: bool,
    pub domain: bool,
    pub port: bool,
    pub resolution: bool,
    pub colors: bool,
}

/// Which attributes of a node were longer than their cap and were cut (see the `MAX_*_CHARS`
/// constants). The mapper turns each into a report line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clipped {
    pub name: bool,
    pub description: bool,
    /// The host was too long to be one; `hostname` is empty then, never a cut-off host.
    pub hostname: bool,
    pub username: bool,
    pub domain: bool,
    /// `protocol` holds only the start of a value that cannot be a supported protocol.
    pub protocol: bool,
}

#[derive(Clone)]
pub struct RawNode {
    pub kind: RawKind,
    pub name: String,
    pub description: String,
    pub hostname: String,
    pub protocol: String,
    pub port: Option<u32>,
    /// The raw `Port` text (shortened) when it is present but not a valid `u32`; `port` is `None` then.
    pub port_invalid: Option<String>,
    pub username: String,
    pub domain: String,
    pub password: Option<Zeroizing<String>>,
    /// The `Password` attribute was present but could not be decrypted.
    pub password_error: bool,
    pub inherit: Inherit,
    pub putty_session: String,
    /// The RDP `Resolution` attribute (cut at `MAX_PROTOCOL_CHARS`; a longer value is not one of
    /// the known values anyway).
    pub resolution: String,
    /// The RDP `Colors` attribute (cut at `MAX_PROTOCOL_CHARS`).
    pub colors: String,
    pub clipped: Clipped,
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
            .field("port_invalid", &self.port_invalid)
            .field("username", &self.username)
            .field("has_password", &self.password.is_some())
            .field("password_error", &self.password_error)
            .field("inherit", &self.inherit)
            .field("resolution", &self.resolution)
            .field("colors", &self.colors)
            .field("children", &self.children)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct RawDocument {
    pub nodes: Vec<RawNode>,
}

fn unsupported(msg: impl Into<String>) -> ImportError {
    ImportError::UnsupportedFormat(crate::limits::cut_chars(
        &msg.into(),
        crate::limits::MAX_MESSAGE_CHARS,
    ))
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
    // before roxmltree: it has no attribute or namespace limit, and both are superlinear
    scan::check(text, MAX_ATTRIBUTES_PER_TAG, MAX_XMLNS_DECLARATIONS)?;
    let opts = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: XML_NODES_LIMIT,
        ..Default::default()
    };
    roxmltree::Document::parse_with_options(text, opts).map_err(|e| match e {
        roxmltree::Error::NodesLimitReached => {
            ImportError::TooComplex("the XML holds too many elements".to_owned())
        }
        _ => unsupported(
            "the file is not a readable XML document (pre-2.6 mRemoteNG files and other formats are not supported)",
        ),
    })
}

fn version_hint(version: &str) -> &'static str {
    let mut parts = version.split('.');
    let major = parts.next().and_then(|m| m.parse::<u32>().ok());
    let minor = parts.next().and_then(|m| m.parse::<u32>().ok());
    match (major, minor) {
        (Some(2), Some(m)) if m >= 7 => {
            "mRemoteNG 2.7 and later are pre-release formats with a separate credential repository and are not supported"
        }
        (Some(m), _) if m < 2 => "legacy files (AES-CBC) are on the backlog",
        (Some(2), Some(m)) if m < 6 => "legacy files (AES-CBC) are on the backlog",
        _ => "this file is not a known mRemoteNG version",
    }
}

fn flag(node: roxmltree::Node, name: &str) -> bool {
    node.attribute(name)
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// An attribute as an owned string of at most `max` characters, and whether it was cut. The cut is
/// found in `max` steps and only the kept part is copied, so a 16 MB value costs nothing here.
fn attr(node: roxmltree::Node, name: &str, max: usize) -> (String, bool) {
    let value = node.attribute(name).unwrap_or_default();
    match value.char_indices().nth(max) {
        None => (value.to_owned(), false),
        Some((end, _)) => (value[..end].to_owned(), true),
    }
}

/// [`attr`] followed by [`crate::limits::clean`] (control and direction characters removed; the
/// cut flag still says whether the raw value was too long).
fn attr_clean(
    node: roxmltree::Node,
    name: &str,
    max: usize,
    keep_newlines: bool,
) -> (String, bool) {
    let (value, cut) = attr(node, name, max);
    (crate::limits::clean(&value, max, keep_newlines), cut)
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

/// Number of stored passwords below `root` (every `Node` element with a non-empty `Password`;
/// a cheap pass that over-counts rather than under-counts).
fn count_passwords(root: roxmltree::Node) -> u64 {
    root.descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Node")
        .filter(|n| {
            n.attribute("Password")
                .is_some_and(|v| !v.trim().is_empty())
        })
        .count() as u64
}

/// Refuses a file whose decryption would cost more than `MAX_PBKDF2_ITERATIONS`: `key_checks`
/// derivations (the `Protected` check and a full-file body) plus one per stored password.
fn check_budget(iterations: u32, key_checks: u64, passwords: u64) -> Result<(), ImportError> {
    let iterations = u64::from(iterations);
    let total = (key_checks + passwords).saturating_mul(iterations);
    if total > MAX_PBKDF2_ITERATIONS {
        return Err(ImportError::TooComplex(format!(
            "decrypting it would take {total} PBKDF2 iterations ({passwords} stored passwords and {key_checks} key check{s}, {iterations} iterations each), more than the {MAX_PBKDF2_ITERATIONS} this importer allows",
            s = if key_checks == 1 { "" } else { "s" }
        )));
    }
    Ok(())
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
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        return Err(unsupported(
            "the file is UTF-16, please re-save it as UTF-8",
        ));
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
            "ConfVersion {} is not supported, only 2.6 ({})",
            shown(version),
            version_hint(version.trim())
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

    let full_file = flag(root, "FullFileEncryption");
    // Before anything is derived: a file without full-file encryption shows its passwords now.
    let key_checks = if full_file { 2 } else { 1 };
    check_budget(iterations, key_checks, count_passwords(root))?;

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
    if !full_file {
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
    // The passwords of the decrypted body, counted before the first one is derived.
    check_budget(
        iterations,
        key_checks,
        count_passwords(inner_xml.root_element()),
    )?;
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
        if depth > MAX_DEPTH {
            return Err(ImportError::TooComplex(format!(
                "folders are nested more than {MAX_DEPTH} levels deep"
            )));
        }
        if *counter > MAX_NODES {
            return Err(ImportError::TooComplex(format!(
                "it holds more than {MAX_NODES} folders and connections"
            )));
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
    // Text that is shown or used later is cleaned of control and direction characters right here
    // (character references can carry them), after the cut and before anything looks at it.
    let (name, name_cut) = attr_clean(el, "Name", MAX_NAME_CHARS, false);
    let (description, description_cut) = attr_clean(el, "Descr", MAX_DESCRIPTION_CHARS, true);
    let (mut hostname, hostname_cut) = attr_clean(el, "Hostname", MAX_HOST_CHARS, false);
    if hostname_cut {
        // too long to be a host: dropped, never cut into a different host
        hostname.clear();
    }
    let (protocol, protocol_cut) = attr(el, "Protocol", MAX_PROTOCOL_CHARS);
    let (username, username_cut) = attr(el, "Username", MAX_IDENTITY_CHARS);
    // like a name: no control characters (a cleaned-empty user name is no user name)
    let username = crate::limits::clean(&username, MAX_IDENTITY_CHARS, false);
    let (domain, domain_cut) = attr_clean(el, "Domain", MAX_IDENTITY_CHARS, false);
    let (putty_session, _) = attr_clean(el, "PuttySession", MAX_IDENTITY_CHARS, false);
    let (resolution, _) = attr(el, "Resolution", MAX_PROTOCOL_CHARS);
    let (colors, _) = attr(el, "Colors", MAX_PROTOCOL_CHARS);
    let kind = match el.attribute("Type").map(str::trim) {
        Some("Container") => RawKind::Container,
        Some("Connection") | None => RawKind::Connection,
        Some(other) => RawKind::Unknown(shown(other)),
    };
    let (port, port_invalid) = match el.attribute("Port").map(str::trim) {
        None | Some("") => (None, None),
        Some(p) => match p.parse::<u32>() {
            Ok(n) => (Some(n), None),
            Err(_) => (None, Some(shown(p))),
        },
    };
    let (password, password_error) = match el.attribute("Password").map(str::trim) {
        None | Some("") => (None, false),
        // far longer than any encrypted password: not worth deriving a key for
        Some(secret) if secret.chars().nth(MAX_PASSWORD_CHARS).is_some() => (None, true),
        Some(secret) => match crypto::decrypt(secret, ctx.key, ctx.iterations) {
            Ok(p) if p.is_empty() => (None, false),
            Ok(p) => (Some(p), false),
            Err(_) => (None, true),
        },
    };
    let children = match kind {
        RawKind::Connection => Vec::new(),
        RawKind::Container | RawKind::Unknown(_) => walk(el, depth + 1, counter, ctx)?,
    };
    Ok(RawNode {
        kind,
        description,
        hostname,
        protocol,
        port,
        port_invalid,
        username,
        domain,
        password,
        password_error,
        inherit: Inherit {
            username: flag(el, "InheritUsername"),
            password: flag(el, "InheritPassword"),
            domain: flag(el, "InheritDomain"),
            port: flag(el, "InheritPort"),
            resolution: flag(el, "InheritResolution"),
            colors: flag(el, "InheritColors"),
        },
        putty_session,
        resolution,
        colors,
        clipped: Clipped {
            name: name_cut,
            description: description_cut,
            hostname: hostname_cut,
            username: username_cut,
            domain: domain_cut,
            protocol: protocol_cut,
        },
        name,
        children,
    })
}
