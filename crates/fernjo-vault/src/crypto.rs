//! Key derivation (Argon2id), AEAD (XChaCha20-Poly1305) and the secret key type.

use std::fmt;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

use crate::error::VaultError;
use crate::format::KdfParams;

/// Size in bytes of keys, salts are `SALT_LEN`, nonces `NONCE_LEN`.
pub(crate) const KEY_LEN: usize = 32;
pub(crate) const NONCE_LEN: usize = 24;
pub(crate) const SALT_LEN: usize = 16;
pub(crate) const TAG_LEN: usize = 16;

/// The random 256-bit key that encrypts the vault body. Zeroised on drop, never printed.
pub struct DataKey([u8; KEY_LEN]);

impl DataKey {
    /// Wraps raw key bytes (for example read back from the OS keychain).
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// The raw key bytes. Handle with care; used for the keychain only.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub(crate) fn random() -> Result<Self, VaultError> {
        let mut bytes = [0u8; KEY_LEN];
        fill_random(&mut bytes)?;
        Ok(Self(bytes))
    }
}

impl Drop for DataKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataKey(<redacted>)")
    }
}

pub(crate) fn fill_random(buf: &mut [u8]) -> Result<(), VaultError> {
    getrandom::fill(buf).map_err(|e| VaultError::SaveFailed(format!("no random source: {e}")))
}

pub(crate) fn random_array<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut out = [0u8; N];
    fill_random(&mut out)?;
    Ok(out)
}

/// Argon2id: password bytes + salt -> 32-byte key-encryption key.
pub(crate) fn derive_key(
    password: &[u8],
    params: KdfParams,
    salt: &[u8; SALT_LEN],
) -> Result<Zeroizing<[u8; KEY_LEN]>, VaultError> {
    let p = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|_| VaultError::InvalidKdfParams)?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password, salt, out.as_mut())
        .map_err(|_| VaultError::InvalidKdfParams)?;
    Ok(out)
}

/// Encrypts `plaintext`; `aad` is authenticated but not encrypted.
pub(crate) fn seal(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, VaultError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .encrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| VaultError::SaveFailed("encryption failed".to_owned()))
}

/// Decrypts and authenticates; `Err(())` for any authentication failure.
pub(crate) fn open(
    key: &[u8; KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, ()> {
    let cipher = XChaCha20Poly1305::new(key.into());
    cipher
        .decrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| ())
}
