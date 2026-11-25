//! The encrypted full backup: connections tree plus vault entries in one password-sealed
//! file. The container is `fernjo_vault::seal_with_password` with magic `FERNJOBK`; the
//! body is JSON (see `Body`). Errors never carry secret text.
//!
//! Limits on the decrypted body: at most `MAX_BODY_BYTES` (64 MiB), at most `MAX_NODES`
//! tree nodes (the same 100 000 as the mRemoteNG importer) and `MAX_ENTRIES` entries.

use std::borrow::Cow;
use std::fmt;

use fernjo_core::json;
use fernjo_core::persist::{document, parse_document_bytes};
use fernjo_core::{DocumentError, MAX_TREE_DEPTH, Tree};
use fernjo_vault::{
    EntryId, KdfParams, NewEntry, SealedError, Secret, open_with_password, seal_with_password,
};
use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, DeserializeSeed, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::limits::{MAX_MESSAGE_CHARS, clean, sanitize_nodes};
use crate::mremoteng::file::MAX_NODES;

pub const MAGIC: &[u8; 8] = b"FERNJOBK";
pub const VERSION: u16 = 1;
const MIN_PASSWORD_CHARS: usize = 8;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

/// Everything a backup holds.
pub struct BackupContents {
    pub created_at: String,
    pub app: String,
    pub tree: Tree,
    pub entries: Vec<(EntryId, NewEntry)>,
}

impl fmt::Debug for BackupContents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupContents")
            .field("created_at", &self.created_at)
            .field("app", &self.app)
            .field("top_level_nodes", &self.tree.children().len())
            .field("entries", &self.entries.len())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    #[error("the backup password must have at least 8 characters")]
    PasswordTooShort,
    #[error("this is not a fernjo backup")]
    NotABackup,
    /// The container, the body or the `connections` document comes from a newer fernjo; `found`
    /// is that format's version number.
    #[error("the backup was written by a newer version of fernjo (format {found})")]
    NewerVersion { found: u16 },
    #[error("wrong password or the file is damaged")]
    WrongPassword,
    #[error("the backup is damaged: {0}")]
    Damaged(String),
}

impl BackupError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PasswordTooShort => "password_too_short",
            Self::NotABackup => "not_a_backup",
            Self::NewerVersion { .. } => "newer_version",
            Self::WrongPassword => "wrong_password",
            Self::Damaged(_) => "damaged",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BodyOut {
    schema_version: u32,
    created_at: String,
    app: String,
    connections: Value,
    entries: Vec<BodyEntry>,
}

/// The body as it is read: `connections` stays the borrowed JSON text until it is counted and
/// parsed, so the document is never built as a `serde_json::Value` (M11 review #28).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BodyIn<'a> {
    schema_version: u32,
    created_at: String,
    app: String,
    #[serde(borrow)]
    connections: &'a RawValue,
    entries: Vec<BodyEntry>,
}

#[derive(Serialize, Deserialize)]
struct BodyEntry {
    id: EntryId,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
    secret: Secret,
    #[serde(default)]
    notes: String,
}

fn damaged(text: &str) -> BackupError {
    BackupError::Damaged(text.to_owned())
}

/// The password rule of a new backup: at least 8 characters after NFC normalisation. `create`
/// applies it; callers can run it earlier (before asking where to save the file).
///
/// # Errors
/// `PasswordTooShort`.
pub fn check_password(password: &SecretString) -> Result<(), BackupError> {
    if password.expose_secret().nfc().count() < MIN_PASSWORD_CHARS {
        return Err(BackupError::PasswordTooShort);
    }
    Ok(())
}

/// Seals `contents` under `password`.
///
/// # Errors
/// `PasswordTooShort` (checked before any key derivation) or `Damaged` when sealing fails.
pub fn create(
    contents: &BackupContents,
    password: &SecretString,
    kdf: KdfParams,
) -> Result<Vec<u8>, BackupError> {
    check_password(password)?;
    let body = BodyOut {
        schema_version: 1,
        created_at: contents.created_at.clone(),
        app: contents.app.clone(),
        connections: document(&contents.tree),
        entries: contents
            .entries
            .iter()
            .map(|(id, e)| BodyEntry {
                id: *id,
                name: e.name.clone(),
                username: e.username.clone(),
                domain: e.domain.clone(),
                secret: e.secret.clone(),
                notes: e.notes.clone(),
            })
            .collect(),
    };
    let failed = || damaged("the backup could not be written");
    let json = Zeroizing::new(serde_json::to_vec(&body).map_err(|_| failed())?);
    seal_with_password(MAGIC, VERSION, password, kdf, &json).map_err(|_| failed())
}

/// True when the `connections` document holds more than `MAX_NODES` nodes. It counts the
/// elements of every `children` array while reading the text and keeps nothing; a document that
/// does not have that shape is left to `parse_document_bytes`.
fn too_many_nodes(document: &RawValue) -> bool {
    let mut count = NodeCount {
        total: 0,
        exceeded: false,
    };
    let mut de = serde_json::Deserializer::from_str(document.get());
    let _ = CountIn { count: &mut count }.deserialize(&mut de);
    count.exceeded
}

struct NodeCount {
    total: usize,
    exceeded: bool,
}

/// Any value: an object's `children` array is counted, everything else skipped.
struct CountIn<'c> {
    count: &'c mut NodeCount,
}

impl<'de> DeserializeSeed<'de> for CountIn<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for CountIn<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<Cow<'de, str>>()? {
            if key == "children" {
                map.next_value_seed(CountChildren { count: self.count })?;
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// The value of a `children` key: every element of an array is one node.
struct CountChildren<'c> {
    count: &'c mut NodeCount,
}

impl<'de> DeserializeSeed<'de> for CountChildren<'_> {
    type Value = ();

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<(), D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for CountChildren<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        loop {
            if self.count.total > MAX_NODES {
                self.count.exceeded = true;
                return Err(de::Error::custom("too many nodes"));
            }
            if seq
                .next_element_seed(CountIn {
                    count: &mut *self.count,
                })?
                .is_none()
            {
                return Ok(());
            }
            self.count.total += 1;
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(())
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

/// Opens a backup.
///
/// # Errors
/// See [`BackupError`]. A body `schemaVersion` above 1, or a `connections` document from a
/// newer fernjo, is reported as `NewerVersion`.
pub fn open(bytes: &[u8], password: &SecretString) -> Result<BackupContents, BackupError> {
    let plain = open_with_password(MAGIC, VERSION, password, bytes).map_err(|e| match e {
        SealedError::NotThisFormat => BackupError::NotABackup,
        SealedError::NewerVersion { found } => BackupError::NewerVersion { found },
        SealedError::Damaged => damaged("the file is truncated or altered"),
        SealedError::WrongPassword => BackupError::WrongPassword,
    })?;
    if plain.len() > MAX_BODY_BYTES {
        return Err(damaged("the backup is too large"));
    }
    let invalid = || damaged("the backup content is not valid");
    // the whole body is validated as a `Value` parse would (also the nesting limit counted from
    // the body, not from the `connections` document) and `schemaVersion` read, keeping nothing
    let [version] = json::head(&plain, ["schemaVersion"]).map_err(|_| invalid())?;
    let version = version
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(invalid)?;
    if version > 1 {
        return Err(BackupError::NewerVersion {
            found: u16::try_from(version).unwrap_or(u16::MAX),
        });
    }
    let body: BodyIn<'_> = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    if body.schema_version != 1 {
        return Err(invalid());
    }
    if body.entries.len() > MAX_ENTRIES {
        return Err(damaged("the backup holds too many entries"));
    }
    if too_many_nodes(body.connections) {
        return Err(damaged("the backup holds too many connections and folders"));
    }
    let mut tree =
        parse_document_bytes(body.connections.get().as_bytes()).map_err(|e| match e {
            DocumentError::NewerSchema { found, .. } => BackupError::NewerVersion {
                found: u16::try_from(found).unwrap_or(u16::MAX),
            },
            DocumentError::Invalid(_) => damaged("the connections in the backup are not valid"),
        })?;
    // A tree that is nested deeper than the core accepts could not be restored (and might not
    // load again): report it here, where it can only mean a damaged or foreign file.
    if tree.depth() > MAX_TREE_DEPTH {
        return Err(BackupError::Damaged(format!(
            "the connections in the backup are nested deeper than {MAX_TREE_DEPTH} levels"
        )));
    }
    // Names, descriptions and hosts get the same caps as every other import path.
    let mut top = tree.children().to_vec();
    if sanitize_nodes(&mut top) != Default::default() {
        tree = Tree::from_children(top);
    }
    let entries = body
        .entries
        .into_iter()
        .map(|e| {
            (
                e.id,
                NewEntry {
                    name: e.name,
                    username: e.username,
                    domain: e.domain,
                    secret: e.secret,
                    notes: e.notes,
                },
            )
        })
        .collect();
    Ok(BackupContents {
        // The header strings are shown in reports: bounded and free of control characters.
        created_at: clean(&body.created_at, MAX_MESSAGE_CHARS, false),
        app: clean(&body.app, MAX_MESSAGE_CHARS, false),
        tree,
        entries,
    })
}
