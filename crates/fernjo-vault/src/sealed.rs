//! Password-sealed blobs: a small container that encrypts bytes under a password.
//!
//! Layout, all integers big endian (like the vault header):
//!
//! ```text
//! offset  size  field
//!      0     8  magic (chosen by the caller)
//!      8     2  format version, u16
//!     10     4  Argon2id memory in KiB, u32
//!     14     4  Argon2id iterations, u32
//!     18     4  Argon2id parallelism, u32
//!     22    16  Argon2id salt
//!     38    24  nonce
//!     62     n  ciphertext + 16 byte tag
//! ```
//!
//! The whole 62-byte header is the associated data of the AEAD, so changing any header
//! byte fails authentication. The password is NFC-normalised like the master password.

use secrecy::SecretString;
use zeroize::Zeroizing;

use crate::crypto::{self, NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::error::VaultError;
use crate::format::KdfParams;
use crate::vault::normalise;

const OFF_VERSION: usize = 8;
const OFF_MEMORY: usize = 10;
const OFF_ITERATIONS: usize = 14;
const OFF_PARALLELISM: usize = 18;
const OFF_SALT: usize = 22;
const OFF_NONCE: usize = OFF_SALT + SALT_LEN;
const HEADER_LEN: usize = OFF_NONCE + NONCE_LEN;

/// Why a sealed blob could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealedError {
    /// The magic does not match.
    NotThisFormat,
    /// The blob has a newer format version than the caller understands.
    NewerVersion { found: u16 },
    /// Too short, or key derivation parameters outside the accepted range.
    Damaged,
    /// Authentication failed: wrong password (or a modified blob).
    WrongPassword,
}

impl std::fmt::Display for SealedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotThisFormat => f.write_str("not a file of the expected format"),
            Self::NewerVersion { found } => write!(f, "newer format version {found}"),
            Self::Damaged => f.write_str("the file is damaged"),
            Self::WrongPassword => f.write_str("wrong password or modified file"),
        }
    }
}

impl std::error::Error for SealedError {}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Encrypts `plaintext` under `password` with a fresh salt and nonce.
///
/// # Errors
/// `invalid_kdf_params` when `kdf` is outside the accepted range, `save_failed` when no
/// random source or encryption is available.
pub fn seal_with_password(
    magic: &[u8; 8],
    version: u16,
    password: &SecretString,
    kdf: KdfParams,
    plaintext: &[u8],
) -> Result<Vec<u8>, VaultError> {
    kdf.validate()?;
    let salt: [u8; SALT_LEN] = crypto::random_array()?;
    let nonce: [u8; NONCE_LEN] = crypto::random_array()?;
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(magic);
    header.extend_from_slice(&version.to_be_bytes());
    header.extend_from_slice(&kdf.memory_kib.to_be_bytes());
    header.extend_from_slice(&kdf.iterations.to_be_bytes());
    header.extend_from_slice(&kdf.parallelism.to_be_bytes());
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    let password = normalise(password);
    let key = crypto::derive_key(password.as_bytes(), kdf, &salt)?;
    let ciphertext = crypto::seal(&key, &nonce, &header, plaintext)?;
    header.extend_from_slice(&ciphertext);
    Ok(header)
}

/// Opens a blob made by [`seal_with_password`]. The parameters in the header are checked
/// against the vault's load bounds before any key is derived.
///
/// # Errors
/// See [`SealedError`].
pub fn open_with_password(
    magic: &[u8; 8],
    max_version: u16,
    password: &SecretString,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SealedError> {
    if bytes.len() < OFF_VERSION + 2 || bytes[..8] != magic[..] {
        return Err(SealedError::NotThisFormat);
    }
    let version = u16::from_be_bytes([bytes[OFF_VERSION], bytes[OFF_VERSION + 1]]);
    if version > max_version {
        return Err(SealedError::NewerVersion { found: version });
    }
    if bytes.len() < HEADER_LEN + TAG_LEN {
        return Err(SealedError::Damaged);
    }
    let kdf = KdfParams {
        memory_kib: be_u32(bytes, OFF_MEMORY),
        iterations: be_u32(bytes, OFF_ITERATIONS),
        parallelism: be_u32(bytes, OFF_PARALLELISM),
    };
    kdf.validate().map_err(|_| SealedError::Damaged)?;
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&bytes[OFF_SALT..OFF_SALT + SALT_LEN]);
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[OFF_NONCE..OFF_NONCE + NONCE_LEN]);
    let password = normalise(password);
    let key =
        crypto::derive_key(password.as_bytes(), kdf, &salt).map_err(|_| SealedError::Damaged)?;
    crypto::open(&key, &nonce, &bytes[..HEADER_LEN], &bytes[HEADER_LEN..])
        .map_err(|()| SealedError::WrongPassword)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAGIC: &[u8; 8] = b"TESTMAGI";

    fn pw(s: &str) -> SecretString {
        SecretString::from(s.to_owned())
    }

    fn sealed(password: &str, data: &[u8]) -> Vec<u8> {
        seal_with_password(MAGIC, 1, &pw(password), KdfParams::TEST, data).unwrap()
    }

    #[test]
    fn round_trip() {
        let blob = sealed("hunter22", b"hello world");
        assert_eq!(blob.len(), HEADER_LEN + 11 + TAG_LEN);
        let out = open_with_password(MAGIC, 1, &pw("hunter22"), &blob).unwrap();
        assert_eq!(out.as_slice(), b"hello world");
    }

    #[test]
    fn wrong_password_is_wrong_password() {
        let blob = sealed("hunter22", b"x");
        assert_eq!(
            open_with_password(MAGIC, 1, &pw("hunter23"), &blob).unwrap_err(),
            SealedError::WrongPassword
        );
    }

    #[test]
    fn flipping_any_header_byte_is_rejected() {
        let blob = sealed("hunter22", b"payload");
        for i in 0..HEADER_LEN {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(
                open_with_password(MAGIC, 1, &pw("hunter22"), &bad).is_err(),
                "byte {i} was accepted"
            );
        }
        let mut bad = blob.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(open_with_password(MAGIC, 1, &pw("hunter22"), &bad).is_err());
    }

    #[test]
    fn truncated_blob_is_damaged() {
        let blob = sealed("hunter22", b"payload");
        for len in [10, 30, HEADER_LEN, HEADER_LEN + TAG_LEN - 1] {
            assert_eq!(
                open_with_password(MAGIC, 1, &pw("hunter22"), &blob[..len]).unwrap_err(),
                SealedError::Damaged,
                "len {len}"
            );
        }
    }

    #[test]
    fn other_magic_is_not_this_format() {
        let blob = sealed("hunter22", b"x");
        assert_eq!(
            open_with_password(b"OTHERMAG", 1, &pw("hunter22"), &blob).unwrap_err(),
            SealedError::NotThisFormat
        );
        assert_eq!(
            open_with_password(MAGIC, 1, &pw("hunter22"), b"").unwrap_err(),
            SealedError::NotThisFormat
        );
    }

    #[test]
    fn newer_version_is_reported_with_found() {
        let blob = seal_with_password(MAGIC, 5, &pw("hunter22"), KdfParams::TEST, b"x").unwrap();
        assert_eq!(
            open_with_password(MAGIC, 1, &pw("hunter22"), &blob).unwrap_err(),
            SealedError::NewerVersion { found: 5 }
        );
    }

    #[test]
    fn kdf_parameters_out_of_bounds_are_damaged_without_deriving() {
        let blob = sealed("hunter22", b"x");
        let start = std::time::Instant::now();
        for (at, value) in [
            (OFF_MEMORY, u32::MAX),
            (OFF_ITERATIONS, u32::MAX),
            (OFF_PARALLELISM, u32::MAX),
            (OFF_ITERATIONS, 0),
        ] {
            let mut bad = blob.clone();
            bad[at..at + 4].copy_from_slice(&value.to_be_bytes());
            assert_eq!(
                open_with_password(MAGIC, 1, &pw("hunter22"), &bad).unwrap_err(),
                SealedError::Damaged
            );
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn password_is_nfc_normalised() {
        let blob = sealed("pass\u{e9}word", b"x");
        let out = open_with_password(MAGIC, 1, &pw("passe\u{301}word"), &blob).unwrap();
        assert_eq!(out.as_slice(), b"x");
    }

    #[test]
    fn out_of_range_kdf_is_refused_when_sealing() {
        let bad = KdfParams {
            memory_kib: 1,
            ..KdfParams::TEST
        };
        assert!(seal_with_password(MAGIC, 1, &pw("hunter22"), bad, b"x").is_err());
    }
}
