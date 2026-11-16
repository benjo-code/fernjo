//! The encrypted full backup: connections tree plus vault entries in one password-sealed
//! file. The container is `fernjo_vault::seal_with_password` with magic `FERNJOBK`; the
//! body is JSON (see `Body`). Errors never carry secret text.
//!
//! Limits on the decrypted body: at most `MAX_BODY_BYTES` (64 MiB), at most `MAX_NODES`
//! tree nodes (the same 100 000 as the mRemoteNG importer) and `MAX_ENTRIES` entries.

use std::fmt;

use fernjo_core::Tree;
use fernjo_core::persist::{document, parse_document};
use fernjo_vault::{
    EntryId, KdfParams, NewEntry, SealedError, Secret, open_with_password, seal_with_password,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

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
    #[error("the backup was written by a newer version of fernjo (backup format {found})")]
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

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    schema_version: u32,
    created_at: String,
    app: String,
    connections: Value,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Probe {
    schema_version: u32,
}

fn damaged(text: &str) -> BackupError {
    BackupError::Damaged(text.to_owned())
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
    if password.expose_secret().nfc().count() < MIN_PASSWORD_CHARS {
        return Err(BackupError::PasswordTooShort);
    }
    let body = Body {
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

/// True when the `connections` document holds more than `MAX_NODES` nodes (no recursion).
fn too_many_nodes(document: &Value) -> bool {
    let mut stack: Vec<&Value> = vec![document];
    let mut total = 0usize;
    while let Some(v) = stack.pop() {
        if let Some(children) = v.get("children").and_then(Value::as_array) {
            total += children.len();
            if total > MAX_NODES {
                return true;
            }
            stack.extend(children);
        }
    }
    false
}

/// Opens a backup.
///
/// # Errors
/// See [`BackupError`]. A body `schemaVersion` above 1 is reported as `NewerVersion`.
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
    let probe: Probe = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    if probe.schema_version > 1 {
        return Err(BackupError::NewerVersion {
            found: u16::try_from(probe.schema_version).unwrap_or(u16::MAX),
        });
    }
    let body: Body = serde_json::from_slice(&plain).map_err(|_| invalid())?;
    if body.schema_version != 1 {
        return Err(invalid());
    }
    if body.entries.len() > MAX_ENTRIES {
        return Err(damaged("the backup holds too many entries"));
    }
    if too_many_nodes(&body.connections) {
        return Err(damaged("the backup holds too many connections and folders"));
    }
    let tree = parse_document(body.connections)
        .map_err(|_| damaged("the connections in the backup are not valid"))?;
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
        created_at: body.created_at,
        app: body.app,
        tree,
        entries,
    })
}
