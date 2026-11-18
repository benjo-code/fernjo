//! Credential entries: the typed body of the vault and the secret-free summary the UI sees.
//!
//! Secrets are `SecretString`s (redacted `Debug`, zeroised on drop). They are stored
//! byte-exact: no trimming, no normalisation, line endings preserved. The only places
//! that call `expose_secret()` are the serde helpers in this file.

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Identifier of a vault entry.
pub type EntryId = Uuid;

/// The secret part of an entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Secret {
    Password {
        #[serde(
            serialize_with = "serialize_secret",
            deserialize_with = "deserialize_secret"
        )]
        password: SecretString,
    },
    PrivateKey {
        #[serde(
            serialize_with = "serialize_secret",
            deserialize_with = "deserialize_secret"
        )]
        key: SecretString,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            serialize_with = "serialize_opt_secret",
            deserialize_with = "deserialize_opt_secret"
        )]
        passphrase: Option<SecretString>,
    },
}

impl Secret {
    #[must_use]
    pub fn kind(&self) -> SecretKind {
        match self {
            Self::Password { .. } => SecretKind::Password,
            Self::PrivateKey { .. } => SecretKind::PrivateKey,
        }
    }
}

/// Which kind of secret an entry holds (safe to show).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecretKind {
    #[serde(rename = "password")]
    Password,
    #[serde(rename = "privateKey")]
    PrivateKey,
}

/// Input for adding an entry.
#[derive(Debug, Clone)]
pub struct NewEntry {
    pub name: String,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub secret: Secret,
    pub notes: String,
}

/// Partial update: `None` keeps the stored value. For `username` and `domain` an empty
/// string clears the field. `secret: Some` replaces the whole secret.
#[derive(Debug, Clone, Default)]
pub struct EntryUpdate {
    pub name: Option<String>,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub secret: Option<Secret>,
    pub notes: Option<String>,
}

/// What lists and lookups return: everything except the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EntrySummary {
    pub id: EntryId,
    pub name: String,
    pub kind: SecretKind,
    pub username: Option<String>,
    pub domain: Option<String>,
    pub notes: String,
}

/// An entry as stored in the encrypted body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub id: EntryId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    pub secret: Secret,
    #[serde(default)]
    pub notes: String,
}

impl Entry {
    pub(crate) fn summary(&self) -> EntrySummary {
        EntrySummary {
            id: self.id,
            name: self.name.clone(),
            kind: self.secret.kind(),
            username: self.username.clone(),
            domain: self.domain.clone(),
            notes: self.notes.clone(),
        }
    }
}

/// Empty optional text means "not set".
pub(crate) fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

fn serialize_secret<S: Serializer>(secret: &SecretString, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(secret.expose_secret())
}

fn serialize_opt_secret<S: Serializer>(
    secret: &Option<SecretString>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match secret {
        Some(secret) => s.serialize_str(secret.expose_secret()),
        None => s.serialize_none(),
    }
}

fn deserialize_secret<'de, D: Deserializer<'de>>(d: D) -> Result<SecretString, D::Error> {
    String::deserialize(d).map(SecretString::from)
}

fn deserialize_opt_secret<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<SecretString>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.map(SecretString::from))
}
