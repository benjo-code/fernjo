//! The hub secret and the sync key of one sync group, in the OS keychain. One item per group
//! (service `fernjo`, user `sync-<group id>`) holds both values, so they are stored, replaced
//! and removed together. Nothing here writes a secret to a file, a log or an error text.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use zeroize::{Zeroize, Zeroizing};

use crate::envelope::{GroupId, SyncKey};
use crate::error::SyncError;

/// Keychain service name of all fernjo items (the same as the vault's).
const SERVICE: &str = "fernjo";

/// Storage for the hub secret and the sync key per group.
pub trait SyncSecrets: Send + Sync {
    /// The stored pair, or `None` when there is no item.
    ///
    /// # Errors
    /// `keychain_unavailable` when the keychain cannot be used or the item is damaged.
    fn get(&self, group: GroupId) -> Result<Option<(SecretString, SyncKey)>, SyncError>;

    /// Stores (or replaces) the pair.
    ///
    /// # Errors
    /// `keychain_unavailable`.
    fn set(
        &self,
        group: GroupId,
        hub_secret: &SecretString,
        key: &SyncKey,
    ) -> Result<(), SyncError>;

    /// Removes the item; a missing item is not an error.
    ///
    /// # Errors
    /// `keychain_unavailable`.
    fn delete(&self, group: GroupId) -> Result<(), SyncError>;
}

/// In-memory store for tests (the app's tests use it too).
#[derive(Default)]
pub struct MemorySyncSecrets {
    items: Mutex<HashMap<GroupId, (SecretString, [u8; 32])>>,
}

impl Drop for MemorySyncSecrets {
    fn drop(&mut self) {
        let items = self
            .items
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (_, key) in items.values_mut() {
            key.zeroize();
        }
    }
}

impl fmt::Debug for MemorySyncSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemorySyncSecrets(<redacted>)")
    }
}

impl SyncSecrets for MemorySyncSecrets {
    fn get(&self, group: GroupId) -> Result<Option<(SecretString, SyncKey)>, SyncError> {
        let items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(items.get(&group).map(|(secret, key)| {
            (
                SecretString::from(secret.expose_secret().to_owned()),
                SyncKey::from_bytes(*key),
            )
        }))
    }

    fn set(
        &self,
        group: GroupId,
        hub_secret: &SecretString,
        key: &SyncKey,
    ) -> Result<(), SyncError> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let value = (
            SecretString::from(hub_secret.expose_secret().to_owned()),
            *key.as_bytes(),
        );
        if let Some((_, mut old)) = items.insert(group, value) {
            old.zeroize();
        }
        Ok(())
    }

    fn delete(&self, group: GroupId) -> Result<(), SyncError> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, mut old)) = items.remove(&group) {
            old.zeroize();
        }
        Ok(())
    }
}

/// The keychain item content: `{"hubSecret":"...","key":"<base64>"}`.
pub(crate) fn encode_item(
    hub_secret: &SecretString,
    key: &SyncKey,
) -> Result<Zeroizing<Vec<u8>>, SyncError> {
    let secret_json = Zeroizing::new(
        serde_json::to_string(hub_secret.expose_secret())
            .map_err(|_| SyncError::KeychainUnavailable("the item cannot be encoded".into()))?,
    );
    let key_text = Zeroizing::new(STANDARD.encode(key.as_bytes()));
    let item = Zeroizing::new(format!(
        "{{\"hubSecret\":{},\"key\":\"{}\"}}",
        *secret_json, *key_text
    ));
    Ok(Zeroizing::new(item.as_bytes().to_vec()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemIn {
    hub_secret: String,
    key: String,
}

impl Drop for ItemIn {
    fn drop(&mut self) {
        self.hub_secret.zeroize();
        self.key.zeroize();
    }
}

/// Reads the pair back; a damaged item is `keychain_unavailable` (the text names no value).
pub(crate) fn decode_item(bytes: &[u8]) -> Result<(SecretString, SyncKey), SyncError> {
    let damaged = || SyncError::KeychainUnavailable("the stored sync item is damaged".into());
    let item: ItemIn = serde_json::from_slice(bytes).map_err(|_| damaged())?;
    let raw = Zeroizing::new(STANDARD.decode(&item.key).map_err(|_| damaged())?);
    let key: [u8; 32] = raw.as_slice().try_into().map_err(|_| damaged())?;
    Ok((
        SecretString::from(item.hub_secret.clone()),
        SyncKey::from_bytes(key),
    ))
}

/// The OS keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux)
/// through `keyring`, the same stack as the vault's data key.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsSyncSecrets;

fn map_err(e: &keyring::Error) -> SyncError {
    // `keyring::Error` describes the platform failure, never the secret.
    SyncError::KeychainUnavailable(e.to_string())
}

fn entry(group: GroupId) -> Result<keyring_core::Entry, SyncError> {
    if let Err(e) = keyring::Entry::store_status() {
        return Err(map_err(e));
    }
    let user = format!("sync-{group}");
    // Windows: ask for "Local" persistence so the item does not roam with a domain profile
    // (the same as the vault's data key).
    #[cfg(windows)]
    let result = keyring_core::Entry::new_with_modifiers(
        SERVICE,
        &user,
        &HashMap::from([("persistence", "Local")]),
    );
    #[cfg(not(windows))]
    let result = keyring_core::Entry::new(SERVICE, &user);
    result.map_err(|e| map_err(&e))
}

impl SyncSecrets for OsSyncSecrets {
    fn get(&self, group: GroupId) -> Result<Option<(SecretString, SyncKey)>, SyncError> {
        match entry(group)?.get_secret() {
            Ok(bytes) => decode_item(&Zeroizing::new(bytes)).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(map_err(&e)),
        }
    }

    fn set(
        &self,
        group: GroupId,
        hub_secret: &SecretString,
        key: &SyncKey,
    ) -> Result<(), SyncError> {
        let item = encode_item(hub_secret, key)?;
        entry(group)?.set_secret(&item).map_err(|e| map_err(&e))
    }

    fn delete(&self, group: GroupId) -> Result<(), SyncError> {
        match entry(group)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(map_err(&e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use base64::engine::general_purpose::STANDARD;
    use secrecy::{ExposeSecret, SecretString};
    use uuid::Uuid;

    use super::*;
    use crate::envelope::SyncKey;

    const SENTINEL_SECRET: &str = "SENTINEL-hub-secret-8841";

    fn key() -> SyncKey {
        SyncKey::from_bytes([0xAB; 32])
    }

    #[test]
    fn memory_secrets_round_trip_and_delete() {
        let store = MemorySyncSecrets::default();
        let group = Uuid::from_u128(1);
        assert!(store.get(group).unwrap().is_none());
        store
            .set(group, &SecretString::from("s1".to_owned()), &key())
            .unwrap();
        let (secret, got) = store.get(group).unwrap().unwrap();
        assert_eq!(secret.expose_secret(), "s1");
        assert_eq!(got.as_bytes(), &[0xAB; 32]);
        store
            .set(group, &SecretString::from("s2".to_owned()), &key())
            .unwrap();
        assert_eq!(store.get(group).unwrap().unwrap().0.expose_secret(), "s2");
        assert!(store.get(Uuid::from_u128(2)).unwrap().is_none());
        store.delete(group).unwrap();
        store.delete(group).unwrap();
        assert!(store.get(group).unwrap().is_none());
    }

    #[test]
    fn debug_of_the_secret_stores_is_redacted() {
        let store = MemorySyncSecrets::default();
        let group = Uuid::from_u128(1);
        let key = SyncKey::from_bytes([0x5A; 32]);
        store
            .set(group, &SecretString::from(SENTINEL_SECRET.to_owned()), &key)
            .unwrap();
        let shown = format!(
            "{store:?} {:?} {:?}",
            OsSyncSecrets,
            store.get(group).unwrap()
        );
        assert!(!shown.contains(SENTINEL_SECRET));
        assert!(!shown.contains(&STANDARD.encode([0x5A; 32])));
        assert!(!shown.contains("90, 90"));
        assert!(!shown.contains("5a5a"));
    }

    #[test]
    fn keychain_item_encoding_round_trips() {
        let secret = SecretString::from("p\"w\\ with \u{e9} and \n".to_owned());
        let item = encode_item(&secret, &key()).unwrap();
        let (got_secret, got_key) = decode_item(&item).unwrap();
        assert_eq!(got_secret.expose_secret(), secret.expose_secret());
        assert_eq!(got_key.as_bytes(), &[0xAB; 32]);
    }

    #[test]
    fn a_damaged_keychain_item_is_keychain_unavailable_and_names_no_value() {
        let bad_key = format!(
            "{{\"hubSecret\":\"{SENTINEL_SECRET}\",\"key\":\"{}\"}}",
            STANDARD.encode([1u8; 5])
        );
        for bytes in [
            &b"not json"[..],
            br#"{"hubSecret":"x"}"#,
            br#"{"hubSecret":"x","key":"!!!"}"#,
            bad_key.as_bytes(),
        ] {
            let err = decode_item(bytes).map(|_| ()).unwrap_err();
            assert_eq!(err.code(), "keychain_unavailable");
            assert!(!err.to_string().contains(SENTINEL_SECRET));
        }
    }
}
