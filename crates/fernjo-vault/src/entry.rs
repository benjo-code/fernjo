//! Credential entries: the typed body of the vault and the secret-free summary the UI sees.
//!
//! Secrets are `SecretString`s (redacted `Debug`, zeroised on drop). They are stored
//! byte-exact: no trimming, no normalisation, line endings preserved. The only places
//! that call `expose_secret()` are the serde helpers in this file.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Unexpected, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;

/// Identifier of a vault entry.
pub type EntryId = Uuid;

/// The secret part of an entry. `Deserialize` is written by hand (see below).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Secret {
    Password {
        #[serde(serialize_with = "serialize_secret")]
        password: SecretString,
    },
    PrivateKey {
        #[serde(serialize_with = "serialize_secret")]
        key: SecretString,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            serialize_with = "serialize_opt_secret"
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

// `Secret` reads by hand: the derived impl of an internally tagged enum buffers every field of
// the object, unknown ones included, before it looks at `type` (M11 review #28: a backup entry
// with junk under an unknown key cost about 45 bytes per input byte). This one reads the fields
// in any order, ignores the other variant's fields and unknown ones without keeping them, and
// accepts what the derived impl accepted for every entry fernjo writes. A repeated key is
// refused, as the derived impl refused it.

#[derive(Deserialize, Clone, Copy)]
#[serde(field_identifier)]
enum SecretKey {
    #[serde(rename = "type")]
    Type,
    #[serde(rename = "password")]
    Password,
    #[serde(rename = "key")]
    Key,
    #[serde(rename = "passphrase")]
    Passphrase,
    #[serde(other)]
    Other,
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum SecretTag {
    Password,
    PrivateKey,
}

/// A secret field read before or after `type`: a string becomes a `SecretString` at once,
/// anything else is kept only as a description for the error.
enum Held {
    Text(SecretString),
    Null,
    Bad(&'static str),
}

impl Held {
    fn required<E: de::Error>(self) -> Result<SecretString, E> {
        match self {
            Held::Text(s) => Ok(s),
            Held::Null => Err(E::invalid_type(Unexpected::Unit, &"a string")),
            Held::Bad(what) => Err(E::invalid_type(Unexpected::Other(what), &"a string")),
        }
    }

    fn optional<E: de::Error>(self) -> Result<Option<SecretString>, E> {
        match self {
            Held::Text(s) => Ok(Some(s)),
            Held::Null => Ok(None),
            Held::Bad(what) => Err(E::invalid_type(Unexpected::Other(what), &"a string")),
        }
    }
}

impl<'de> Deserialize<'de> for Held {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(HeldVisitor)
    }
}

struct HeldVisitor;

impl<'de> Visitor<'de> for HeldVisitor {
    type Value = Held;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Held, E> {
        Ok(Held::Text(SecretString::from(v.to_owned())))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Held, E> {
        Ok(Held::Text(SecretString::from(v)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Held, E> {
        Ok(Held::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Held, E> {
        Ok(Held::Null)
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Held, E> {
        Ok(Held::Bad("boolean"))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Held, E> {
        Ok(Held::Bad("integer"))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Held, E> {
        Ok(Held::Bad("integer"))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Held, E> {
        Ok(Held::Bad("floating point"))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Held, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Held::Bad("sequence"))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Held, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(Held::Bad("map"))
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_map(SecretVisitor)
    }
}

struct SecretVisitor;

impl<'de> Visitor<'de> for SecretVisitor {
    type Value = Secret;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a secret object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Secret, A::Error> {
        let mut tag = None;
        let mut password = None;
        let mut key = None;
        let mut passphrase = None;
        while let Some(field) = map.next_key::<SecretKey>()? {
            let (repeated, name) = match field {
                SecretKey::Type => (tag.is_some(), "type"),
                SecretKey::Password => (password.is_some(), "password"),
                SecretKey::Key => (key.is_some(), "key"),
                SecretKey::Passphrase => (passphrase.is_some(), "passphrase"),
                SecretKey::Other => (false, ""),
            };
            if repeated {
                return Err(de::Error::duplicate_field(name));
            }
            match (field, tag) {
                (SecretKey::Type, _) => tag = Some(map.next_value::<SecretTag>()?),
                (SecretKey::Password, Some(SecretTag::PrivateKey))
                | (SecretKey::Key | SecretKey::Passphrase, Some(SecretTag::Password))
                | (SecretKey::Other, _) => {
                    map.next_value::<IgnoredAny>()?;
                }
                (SecretKey::Password, _) => password = Some(map.next_value::<Held>()?),
                (SecretKey::Key, _) => key = Some(map.next_value::<Held>()?),
                (SecretKey::Passphrase, _) => passphrase = Some(map.next_value::<Held>()?),
            }
        }
        match tag.ok_or_else(|| de::Error::missing_field("type"))? {
            SecretTag::Password => Ok(Secret::Password {
                password: password
                    .ok_or_else(|| de::Error::missing_field("password"))?
                    .required()?,
            }),
            SecretTag::PrivateKey => Ok(Secret::PrivateKey {
                key: key
                    .ok_or_else(|| de::Error::missing_field("key"))?
                    .required()?,
                passphrase: match passphrase {
                    Some(held) => held.optional()?,
                    None => None,
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Secret, serde_json::Error> {
        serde_json::from_str(text)
    }

    fn shown(secret: &Secret) -> (SecretKind, String, Option<String>) {
        match secret {
            Secret::Password { password } => (
                SecretKind::Password,
                password.expose_secret().to_owned(),
                None,
            ),
            Secret::PrivateKey { key, passphrase } => (
                SecretKind::PrivateKey,
                key.expose_secret().to_owned(),
                passphrase.as_ref().map(|p| p.expose_secret().to_owned()),
            ),
        }
    }

    #[test]
    fn a_secret_reads_its_fields_in_any_order_and_round_trips() {
        let password = Secret::Password {
            password: SecretString::from("pw\n".to_owned()),
        };
        let key = Secret::PrivateKey {
            key: SecretString::from("k".to_owned()),
            passphrase: Some(SecretString::from("pp".to_owned())),
        };
        for secret in [&password, &key] {
            let back = read(&serde_json::to_string(secret).unwrap()).unwrap();
            assert_eq!(shown(&back), shown(secret));
            let via_value: Secret =
                serde_json::from_value(serde_json::to_value(secret).unwrap()).unwrap();
            assert_eq!(shown(&via_value), shown(secret));
        }
        for (text, expected) in [
            (
                r#"{"password":"pw\n","type":"password","x":[1]}"#,
                &password,
            ),
            (
                r#"{"passphrase":"pp","key":"k","type":"privateKey","password":5}"#,
                &key,
            ),
            (
                r#"{"type":"privateKey","key":"k","passphrase":"pp","password":{}}"#,
                &key,
            ),
        ] {
            assert_eq!(shown(&read(text).unwrap()), shown(expected), "{text}");
        }
        let no_passphrase = read(r#"{"type":"privateKey","key":"k","passphrase":null}"#).unwrap();
        assert_eq!(shown(&no_passphrase).2, None);
    }

    #[test]
    fn a_secret_refuses_what_the_derived_impl_refused() {
        for text in [
            r#"{"password":"pw"}"#,
            r#"{"type":"token","password":"pw"}"#,
            r#"{"type":"password"}"#,
            r#"{"type":"password","password":null}"#,
            r#"{"password":5,"type":"password"}"#,
            r#"{"type":"privateKey","passphrase":"pp"}"#,
            r#"{"type":"privateKey","key":"k","passphrase":5}"#,
            r#"{"type":"password","password":"a","password":"b"}"#,
            r#"{"type":"password","type":"privateKey","key":"k"}"#,
        ] {
            assert!(read(text).is_err(), "{text}");
        }
    }

    /// Behaviour that differs from the derived (internally tagged) impl on purpose (M11 review #28,
    /// `Docs/DECISIONS.md`): the old impl read a sequence `["password","pw"]` (its tag as first
    /// element); the hand-written one only reads maps.
    #[test]
    fn a_secret_no_longer_reads_the_sequence_form_the_derived_impl_accepted() {
        assert!(read(r#"["password","pw"]"#).is_err());
    }
}
