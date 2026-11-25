//! Reading JSON files without building a `serde_json::Value` (M11 review #28).
//!
//! A `Value` costs about 98 bytes per input byte for `[{"":0},…]` (every one-key object becomes
//! a whole map), so a hostile 64 MiB `connections.json` cost about 6 GiB before its shape was
//! even looked at. Every reader of an untrusted or size-capped document therefore works in two
//! steps:
//!
//! 1. [`head`] validates the whole document exactly as `serde_json::from_slice::<Value>` would
//!    (syntax, UTF-8 inside strings, serde_json's nesting limit of 128, nothing after the value)
//!    without keeping any of it, and returns the few top-level fields the reader decides on
//!    (a version number, a format marker).
//! 2. The reader deserialises the same bytes straight into its typed structs, which skip unknown
//!    fields with `IgnoredAny` (no allocation). Nested documents that must be read later are
//!    borrowed as `&RawValue`.
//!
//! Types that serde would buffer as `Content` (internally tagged enums, `#[serde(flatten)]`) hold
//! every unknown field in memory too, so the file types implement `Deserialize` by hand instead
//! (`Node`, `RdpResolution`, the vault's `Secret`).

use std::fmt;

use serde::de::{self, DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

/// A top-level field as [`head`] saw it: the cases callers decide on, nothing else is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TopField {
    /// The document is not an object or has no such key.
    Missing,
    /// A non-negative integer (what `Value::as_u64` returns).
    U64(u64),
    /// A string.
    Str(String),
    /// Any other value (negative or fractional number, boolean, null, array, object).
    Other,
}

impl TopField {
    /// The value as `Value::as_u64` would give it.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            TopField::U64(v) => Some(*v),
            _ => None,
        }
    }

    /// The value as `Value::as_str` would give it.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            TopField::Str(s) => Some(s),
            _ => None,
        }
    }
}

/// Validates `bytes` as one JSON document, accepting and refusing exactly what
/// `serde_json::from_slice::<serde_json::Value>` does, and returns the top-level fields named in
/// `keys` (a repeated key: the last one wins, as in a `Value`). Nothing else is kept, so the
/// memory used does not grow with the document.
///
/// # Errors
/// The `serde_json::Error` the `Value` parse would have given (syntax, nesting limit, trailing
/// characters).
pub fn head<const N: usize>(
    bytes: &[u8],
    keys: [&str; N],
) -> Result<[TopField; N], serde_json::Error> {
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let fields = HeadSeed { keys: &keys }.deserialize(&mut de)?;
    de.end()?;
    Ok(fields)
}

/// Validates `bytes` as one JSON document the way [`head`] does, keeping nothing.
///
/// # Errors
/// See [`head`].
pub fn validate(bytes: &[u8]) -> Result<(), serde_json::Error> {
    head(bytes, []).map(|[]| ())
}

struct HeadSeed<'k, const N: usize> {
    keys: &'k [&'k str; N],
}

impl<'de, const N: usize> DeserializeSeed<'de> for HeadSeed<'_, N> {
    type Value = [TopField; N];

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de, const N: usize> Visitor<'de> for HeadSeed<'_, N> {
    type Value = [TopField; N];

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut fields = std::array::from_fn(|_| TopField::Missing);
        while let Some(index) = map.next_key_seed(KeySeed { keys: self.keys })? {
            match index {
                Some(i) => fields[i] = map.next_value::<TopField>()?,
                None => map.next_value::<Skip>().map(|_| ())?,
            }
        }
        Ok(fields)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
        drain_seq(seq)?;
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(std::array::from_fn(|_| TopField::Missing))
    }
}

/// Which of the wanted keys a map key is, without allocating for it.
struct KeySeed<'k, const N: usize> {
    keys: &'k [&'k str; N],
}

impl<'de, const N: usize> DeserializeSeed<'de> for KeySeed<'_, N> {
    type Value = Option<usize>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
        d.deserialize_str(self)
    }
}

impl<'de, const N: usize> Visitor<'de> for KeySeed<'_, N> {
    type Value = Option<usize>;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a string key")
    }

    fn visit_str<E: de::Error>(self, key: &str) -> Result<Self::Value, E> {
        Ok(self.keys.iter().position(|k| *k == key))
    }
}

impl<'de> de::Deserialize<'de> for TopField {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(FieldVisitor)
    }
}

struct FieldVisitor;

impl<'de> Visitor<'de> for FieldVisitor {
    type Value = TopField;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<TopField, E> {
        Ok(TopField::U64(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<TopField, E> {
        // a `Value` stores a non-negative `i64` as a positive integer
        Ok(u64::try_from(v).map_or(TopField::Other, TopField::U64))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<TopField, E> {
        Ok(TopField::Other)
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<TopField, E> {
        Ok(TopField::Other)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<TopField, E> {
        Ok(TopField::Str(v.to_owned()))
    }

    fn visit_unit<E: de::Error>(self) -> Result<TopField, E> {
        Ok(TopField::Other)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<TopField, A::Error> {
        drain_seq(seq)?;
        Ok(TopField::Other)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<TopField, A::Error> {
        drain_map(map)?;
        Ok(TopField::Other)
    }
}

/// Any JSON value, read the way a `Value` is read (`deserialize_any`, so serde_json checks UTF-8
/// inside strings and counts the nesting) but not kept. `IgnoredAny` is not enough here:
/// serde_json skips it without either check.
struct Skip;

impl<'de> de::Deserialize<'de> for Skip {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(SkipVisitor)
    }
}

struct SkipVisitor;

impl<'de> Visitor<'de> for SkipVisitor {
    type Value = Skip;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_unit<E: de::Error>(self) -> Result<Skip, E> {
        Ok(Skip)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Skip, A::Error> {
        drain_seq(seq)?;
        Ok(Skip)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Skip, A::Error> {
        drain_map(map)?;
        Ok(Skip)
    }
}

fn drain_seq<'de, A: SeqAccess<'de>>(mut seq: A) -> Result<(), A::Error> {
    while seq.next_element::<Skip>()?.is_some() {}
    Ok(())
}

fn drain_map<'de, A: MapAccess<'de>>(mut map: A) -> Result<(), A::Error> {
    while map.next_entry::<Skip, Skip>()?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// What the `Value` parse says about `text`: the error text, or the two fields.
    fn by_value(text: &str) -> Result<(Option<u64>, Option<String>), String> {
        let v: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        Ok((
            v.get("v").and_then(Value::as_u64),
            v.get("s").and_then(Value::as_str).map(str::to_owned),
        ))
    }

    fn by_head(text: &str) -> Result<(Option<u64>, Option<String>), String> {
        let [v, s] = head(text.as_bytes(), ["v", "s"]).map_err(|e| e.to_string())?;
        Ok((v.as_u64(), s.as_str().map(str::to_owned)))
    }

    #[test]
    fn head_agrees_with_the_value_parse() {
        let deep_ok = format!("{}{}", "[".repeat(127), "]".repeat(127));
        let deep_bad = format!("{}{}", "[".repeat(128), "]".repeat(128));
        let deep_key = format!(r#"{{"v":1,"x":{}{}}}"#, "[".repeat(127), "]".repeat(127));
        let cases = [
            r#"{"v":3,"s":"fernjo"}"#,
            r#"{"v":3,"v":4}"#,
            r#"{"v":-0}"#,
            r#"{"v":-1,"s":5}"#,
            r#"{"v":3.0,"s":null}"#,
            r#"{"v":"3","s":["x"]}"#,
            r#"{"v":18446744073709551615}"#,
            r#"{"v":18446744073709551616}"#,
            r#"{"v":1e400}"#,
            r#"[{"v":3}]"#,
            "3",
            r#""v""#,
            "null",
            "",
            "{",
            r#"{"v":3} x"#,
            r#"{"v":3,}"#,
            r#"{"x":"\q"}"#,
            r#"{"x":"\ud800"}"#,
            "{\"x\":\"a\u{1}\"}",
            &deep_ok,
            &deep_bad,
            &deep_key,
        ];
        for text in cases {
            assert_eq!(by_head(text), by_value(text), "{text}");
        }
    }

    #[test]
    fn head_refuses_invalid_utf8_inside_an_unknown_string_like_the_value_parse() {
        let bytes = b"{\"v\":1,\"x\":\"\xff\"}";
        let by_value = serde_json::from_slice::<Value>(bytes)
            .unwrap_err()
            .to_string();
        assert_eq!(head(bytes, ["v"]).unwrap_err().to_string(), by_value);
        assert!(validate(bytes).is_err());
        assert!(validate(br#"{"v":1}"#).is_ok());
    }
}
