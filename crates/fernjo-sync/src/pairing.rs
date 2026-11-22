//! The pairing code: everything a second device needs to join a sync group, as one pasteable
//! text. It contains the hub secret and the sync key, so it is shown only on explicit request
//! (`sync_pairing_code`) and never logged; `Debug` is redacted.
//!
//! Format: `fernjo-sync-1:` + base64url (no padding) of `JSON ++ first 4 bytes of SHA-256(JSON)`.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::client::check_hub_url;
use crate::envelope::{GroupId, SyncKey};
use crate::error::SyncError;

const PREFIX: &str = "fernjo-sync-";
const PAYLOAD_VERSION: u32 = 1;
const CHECKSUM_LEN: usize = 4;
/// A code is a few hundred bytes (a CA certificate at most a few KiB); anything above this is
/// not one.
const MAX_CODE_LEN: usize = 64 * 1024;

/// What a pairing code carries.
pub struct PairingCode {
    pub url: String,
    pub secret: SecretString,
    pub key: SyncKey,
    pub group: GroupId,
    pub ca_pem: Option<String>,
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingCode")
            .field("url", &self.url)
            .field("secret", &"<redacted>")
            .field("key", &"<redacted>")
            .field("group", &self.group)
            .field("ca_pem", &self.ca_pem.as_ref().map(|_| "<certificate>"))
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct Payload {
    v: u32,
    url: String,
    secret: String,
    key: String,
    group: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ca: Option<String>,
}

impl PairingCode {
    /// The shareable text. Contains the secrets: only for an explicit "show pairing code".
    #[must_use]
    pub fn encode(&self) -> String {
        let payload = Payload {
            v: PAYLOAD_VERSION,
            url: self.url.clone(),
            secret: self.secret.expose_secret().to_owned(),
            key: URL_SAFE_NO_PAD.encode(self.key.as_bytes()),
            group: self.group.to_string(),
            ca: self.ca_pem.clone(),
        };
        let json = Zeroizing::new(serde_json::to_vec(&payload).expect("plain data serialises"));
        let mut raw = Zeroizing::new(json.to_vec());
        raw.extend_from_slice(&Sha256::digest(&json)[..CHECKSUM_LEN]);
        format!(
            "{PREFIX}{PAYLOAD_VERSION}:{}",
            URL_SAFE_NO_PAD.encode(&*raw)
        )
    }

    /// Parses a pasted code. Whitespace and line breaks anywhere are ignored. Nothing is
    /// trusted before the checksum, the JSON and the hub address have been checked.
    pub fn decode(text: &str) -> Result<PairingCode, SyncError> {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.len() > MAX_CODE_LEN {
            return Err(SyncError::InvalidPairingCode);
        }
        let rest = compact
            .strip_prefix(PREFIX)
            .ok_or(SyncError::InvalidPairingCode)?;
        let (version, body) = rest.split_once(':').ok_or(SyncError::InvalidPairingCode)?;
        if version.is_empty()
            || !version.bytes().all(|b| b.is_ascii_digit())
            || (version.len() > 1 && version.starts_with('0'))
        {
            return Err(SyncError::InvalidPairingCode);
        }
        match version.parse::<u32>() {
            Ok(1) => {}
            Ok(0) => return Err(SyncError::InvalidPairingCode),
            _ => return Err(SyncError::NewerPairingCode),
        }
        let raw = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(body)
                .map_err(|_| SyncError::InvalidPairingCode)?,
        );
        if raw.len() <= CHECKSUM_LEN {
            return Err(SyncError::InvalidPairingCode);
        }
        let (json, checksum) = raw.split_at(raw.len() - CHECKSUM_LEN);
        if Sha256::digest(json)[..CHECKSUM_LEN] != *checksum {
            return Err(SyncError::InvalidPairingCode);
        }
        let payload: Payload =
            serde_json::from_slice(json).map_err(|_| SyncError::InvalidPairingCode)?;
        if payload.v != PAYLOAD_VERSION
            || payload.url.is_empty()
            || payload.secret.is_empty()
            || payload.ca.as_deref().is_some_and(str::is_empty)
        {
            return Err(SyncError::InvalidPairingCode);
        }
        check_hub_url(&payload.url).map_err(|_| SyncError::InvalidPairingCode)?;
        let key_bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(&payload.key)
                .map_err(|_| SyncError::InvalidPairingCode)?,
        );
        let key: [u8; 32] = key_bytes
            .as_slice()
            .try_into()
            .map_err(|_| SyncError::InvalidPairingCode)?;
        let group: GroupId = payload
            .group
            .parse()
            .map_err(|_| SyncError::InvalidPairingCode)?;
        Ok(PairingCode {
            url: payload.url,
            secret: SecretString::from(payload.secret),
            key: SyncKey::from_bytes(key),
            group,
            ca_pem: payload.ca,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn sample(ca: Option<&str>) -> PairingCode {
        PairingCode {
            url: "https://hub.example.test:8443/".to_owned(),
            secret: SecretString::from("hub-secret-sentinel".to_owned()),
            key: SyncKey::from_bytes([7u8; 32]),
            group: GroupId::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0),
            ca_pem: ca.map(str::to_owned),
        }
    }

    const PEM: &str = "-----BEGIN CERTIFICATE-----\nMIIBdummy\n-----END CERTIFICATE-----\n";

    #[test]
    fn encode_decode_round_trips_with_and_without_a_ca() {
        for ca in [None, Some(PEM)] {
            let code = sample(ca);
            let text = code.encode();
            assert!(text.starts_with("fernjo-sync-1:"));
            let back = PairingCode::decode(&text).expect("decodes");
            assert_eq!(back.url, code.url);
            assert_eq!(back.secret.expose_secret(), code.secret.expose_secret());
            assert_eq!(back.key.as_bytes(), code.key.as_bytes());
            assert_eq!(back.group, code.group);
            assert_eq!(back.ca_pem.as_deref(), ca);
        }
    }

    #[test]
    fn a_truncated_code_is_invalid_pairing_code() {
        let text = sample(Some(PEM)).encode();
        for cut in [0, 5, 14, 20, text.len() / 2, text.len() - 1] {
            let err = PairingCode::decode(&text[..cut]).unwrap_err();
            assert_eq!(err.code(), "invalid_pairing_code", "cut at {cut}");
        }
    }

    #[test]
    fn one_changed_character_is_invalid_pairing_code() {
        let text = sample(None).encode();
        let prefix = "fernjo-sync-1:".len();
        for i in (prefix..text.len()).step_by(7) {
            let mut bytes = text.clone().into_bytes();
            bytes[i] = if bytes[i] == b'A' { b'B' } else { b'A' };
            let changed = String::from_utf8(bytes).unwrap();
            let err = PairingCode::decode(&changed).unwrap_err();
            assert_eq!(err.code(), "invalid_pairing_code", "position {i}");
        }
    }

    #[test]
    fn a_code_with_line_breaks_and_spaces_still_decodes() {
        let text = sample(Some(PEM)).encode();
        let mut spaced = String::from("  ");
        for (i, c) in text.chars().enumerate() {
            spaced.push(c);
            if i % 20 == 19 {
                spaced.push_str("\r\n  ");
            } else if i % 7 == 6 {
                spaced.push(' ');
            }
        }
        spaced.push('\n');
        assert!(PairingCode::decode(&spaced).is_ok());
    }

    #[test]
    fn prefix_with_a_higher_version_is_newer_pairing_code() {
        let text = sample(None).encode();
        let rest = text.strip_prefix("fernjo-sync-1:").unwrap();
        for v in ["2", "3", "10", "99"] {
            let err = PairingCode::decode(&format!("fernjo-sync-{v}:{rest}")).unwrap_err();
            assert_eq!(err.code(), "newer_pairing_code", "version {v}");
        }
        for bad in [
            "fernjo-sync-0:",
            "fernjo-sync-:",
            "fernjo-sync-x:",
            "other-1:",
            "",
        ] {
            let err = PairingCode::decode(&format!("{bad}{rest}")).unwrap_err();
            assert_eq!(err.code(), "invalid_pairing_code", "prefix {bad}");
        }
    }

    #[test]
    fn debug_is_redacted() {
        let code = sample(Some(PEM));
        let shown = format!("{code:?}");
        assert!(!shown.contains("hub-secret-sentinel"));
        assert!(!shown.contains("BwcHBwcH")); // the key bytes in base64
        assert!(!shown.contains("7, 7, 7"));
        assert!(shown.contains("hub.example.test"));
    }

    #[test]
    fn a_code_with_a_forbidden_url_is_refused_before_use() {
        let mut code = sample(None);
        code.url = "http://hub.example.test/".to_owned();
        let err = PairingCode::decode(&code.encode()).unwrap_err();
        assert_eq!(err.code(), "invalid_pairing_code");
        code.url = "http://127.0.0.1:8787".to_owned();
        assert!(PairingCode::decode(&code.encode()).is_ok());
    }

    #[test]
    fn empty_fields_are_invalid() {
        let mut code = sample(None);
        code.secret = SecretString::from(String::new());
        assert_eq!(
            PairingCode::decode(&code.encode()).unwrap_err().code(),
            "invalid_pairing_code"
        );
    }
}
