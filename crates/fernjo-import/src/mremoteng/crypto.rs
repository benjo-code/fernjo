//! mRemoteNG AES-GCM secret decryption.
//!
//! Layout: `base64(salt[16] || nonce[16] || ciphertext || tag[16])`. AES-256-GCM with a 16-byte nonce,
//! the salt as associated data; key = PBKDF2-HMAC-SHA1 over the UTF-8 password bytes (32 bytes).

use aes_gcm::AesGcm;
use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes256;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret, SecretString};
use sha1::Sha1;
use zeroize::Zeroizing;

pub const DEFAULT_KEY: &str = "mR3m";
pub const MIN_ITERATIONS: u32 = 1000;
pub const MAX_ITERATIONS: u32 = 1_000_000;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 16;
const TAG_LEN: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    #[error("not valid base64")]
    BadBase64,
    #[error("too short to be an encrypted secret")]
    TooShort,
    #[error("authentication failed")]
    Authentication,
}

/// Decrypts one mRemoteNG secret. Iteration counts outside `MIN_ITERATIONS..=MAX_ITERATIONS` are
/// refused as `Authentication` before any key derivation, so a hostile file cannot make the
/// importer spin.
pub fn decrypt(
    secret_b64: &str,
    key: &SecretString,
    iterations: u32,
) -> Result<Zeroizing<String>, CryptoError> {
    let raw = Zeroizing::new(
        STANDARD
            .decode(secret_b64.trim())
            .map_err(|_| CryptoError::BadBase64)?,
    );
    if raw.len() < SALT_LEN + NONCE_LEN + TAG_LEN {
        return Err(CryptoError::TooShort);
    }
    if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
        return Err(CryptoError::Authentication);
    }
    let (salt, rest) = raw.split_at(SALT_LEN);
    let (nonce, ciphertext) = rest.split_at(NONCE_LEN);

    let mut derived = Zeroizing::new([0u8; 32]);
    pbkdf2::pbkdf2_hmac::<Sha1>(
        key.expose_secret().as_bytes(),
        salt,
        iterations,
        derived.as_mut(),
    );

    let cipher = AesGcm::<Aes256, U16>::new_from_slice(derived.as_ref())
        .map_err(|_| CryptoError::Authentication)?;
    let plain = cipher
        .decrypt(
            nonce.try_into().map_err(|_| CryptoError::Authentication)?,
            Payload {
                msg: ciphertext,
                aad: salt,
            },
        )
        .map_err(|_| CryptoError::Authentication)?;
    let plain = Zeroizing::new(plain);
    String::from_utf8(plain.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| CryptoError::Authentication)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use secrecy::SecretString;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vector {
        password: String,
        iterations: u32,
        plaintext: String,
        secret: String,
    }

    fn vectors() -> Vec<Vector> {
        let raw = include_str!("../../tests/fixtures/vectors.json");
        serde_json::from_str(raw).unwrap()
    }

    fn key(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    #[test]
    fn decrypts_every_independent_vector() {
        let vs = vectors();
        assert!(vs.len() >= 4);
        for v in vs {
            let out = decrypt(&v.secret, &key(&v.password), v.iterations).unwrap();
            assert_eq!(*out, v.plaintext);
        }
    }

    #[test]
    fn first_vector_uses_the_default_key() {
        assert_eq!(vectors()[0].password, DEFAULT_KEY);
    }

    #[test]
    fn wrong_key_is_authentication_error() {
        let v = &vectors()[0];
        assert_eq!(
            decrypt(&v.secret, &key("nope"), v.iterations).unwrap_err(),
            CryptoError::Authentication
        );
    }

    #[test]
    fn flipped_ciphertext_byte_is_authentication_error() {
        let v = &vectors()[0];
        let mut raw = STANDARD.decode(&v.secret).unwrap();
        raw[33] ^= 1;
        let tampered = STANDARD.encode(raw);
        assert_eq!(
            decrypt(&tampered, &key(&v.password), v.iterations).unwrap_err(),
            CryptoError::Authentication
        );
    }

    #[test]
    fn too_short_input_is_too_short() {
        let short = STANDARD.encode([0u8; 47]);
        assert_eq!(
            decrypt(&short, &key("mR3m"), 1000).unwrap_err(),
            CryptoError::TooShort
        );
    }

    #[test]
    fn bad_base64_is_bad_base64() {
        assert_eq!(
            decrypt("not base64 !!!", &key("mR3m"), 1000).unwrap_err(),
            CryptoError::BadBase64
        );
    }

    #[test]
    fn iterations_below_1000_or_above_1_000_000_are_refused_before_deriving() {
        let v = &vectors()[0];
        for it in [0, 999, 1_000_001, u32::MAX] {
            let started = std::time::Instant::now();
            assert_eq!(
                decrypt(&v.secret, &key(&v.password), it).unwrap_err(),
                CryptoError::Authentication,
                "iterations {it}"
            );
            assert!(
                started.elapsed().as_millis() < 50,
                "iterations {it} derived"
            );
        }
    }
}
