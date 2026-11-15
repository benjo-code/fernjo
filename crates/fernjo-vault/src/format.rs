//! Byte layout of `vault.fernjo` (documented in `Docs/vault-format.md`).
//!
//! ```text
//! offset  size  field
//!      0     8  magic "FERNJOVT"
//!      8     2  format version, u16 big endian
//!     10    16  vault id (UUID bytes)
//!     26     4  Argon2id memory in KiB, u32 big endian
//!     30     4  Argon2id iterations, u32 big endian
//!     34     4  Argon2id parallelism, u32 big endian
//!     38    16  Argon2id salt
//!     54    24  nonce of the key wrap
//!     78    48  data key wrapped by the password-derived key (32 + 16 tag)
//!    126    24  nonce of the body
//!    150     n  body ciphertext (JSON + 16 byte tag)
//! ```

use uuid::Uuid;

use crate::crypto::{NONCE_LEN, SALT_LEN, TAG_LEN};
use crate::error::VaultError;

/// Name of the vault file inside the data folder.
pub const FILE_NAME: &str = "vault.fernjo";
/// Format version this build writes and understands.
pub(crate) const CURRENT_VERSION: u16 = 1;
pub(crate) const MAGIC: &[u8; 8] = b"FERNJOVT";

pub(crate) const OFF_VERSION: usize = 8;
pub(crate) const OFF_ID: usize = 10;
pub(crate) const OFF_KDF_MEMORY: usize = 26;
pub(crate) const OFF_KDF_ITERATIONS: usize = 30;
pub(crate) const OFF_KDF_PARALLELISM: usize = 34;
pub(crate) const OFF_SALT: usize = 38;
pub(crate) const OFF_WRAP_NONCE: usize = 54;
pub(crate) const OFF_WRAPPED_KEY: usize = 78;
pub(crate) const WRAPPED_KEY_LEN: usize = 32 + TAG_LEN;
pub(crate) const OFF_BODY_NONCE: usize = OFF_WRAPPED_KEY + WRAPPED_KEY_LEN;
/// Length of the header; the body ciphertext follows.
pub(crate) const HEADER_LEN: usize = OFF_BODY_NONCE + NONCE_LEN;

/// Bounds applied to the parameters read from a file, so a hostile synced file cannot
/// demand gigabytes of memory or minutes of CPU.
const MAX_MEMORY_KIB: u32 = 1024 * 1024; // 1 GiB
const MAX_ITERATIONS: u32 = 20;
const MAX_PARALLELISM: u32 = 16;

/// Argon2id cost parameters. They are stored in the file header, so they can be raised
/// later without breaking existing vaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl KdfParams {
    /// 128 MiB, 6 passes, 4 lanes: measured at about 0.52 s on a 6-core desktop (release
    /// build); see `Docs/DECISIONS.md` for the reasoning.
    pub const DEFAULT: Self = Self {
        memory_kib: 128 * 1024,
        iterations: 6,
        parallelism: 4,
    };

    /// The smallest values the load bounds accept. **Insecure; for tests only.**
    pub const TEST: Self = Self {
        memory_kib: 8,
        iterations: 1,
        parallelism: 1,
    };

    pub(crate) fn validate(self) -> Result<(), VaultError> {
        let ok = (1..=MAX_PARALLELISM).contains(&self.parallelism)
            && (1..=MAX_ITERATIONS).contains(&self.iterations)
            && (8 * self.parallelism..=MAX_MEMORY_KIB).contains(&self.memory_kib);
        if ok {
            Ok(())
        } else {
            Err(VaultError::InvalidKdfParams)
        }
    }
}

impl Default for KdfParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The parsed fixed-size part of the file.
#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub id: Uuid,
    pub kdf: KdfParams,
    pub salt: [u8; SALT_LEN],
    pub wrap_nonce: [u8; NONCE_LEN],
    pub wrapped_key: [u8; WRAPPED_KEY_LEN],
    pub body_nonce: [u8; NONCE_LEN],
}

fn array<const N: usize>(bytes: &[u8], at: usize) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(&bytes[at..at + N]);
    out
}

fn be_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(array(bytes, at))
}

impl Header {
    pub(crate) fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[..8].copy_from_slice(MAGIC);
        out[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&CURRENT_VERSION.to_be_bytes());
        out[OFF_ID..OFF_ID + 16].copy_from_slice(self.id.as_bytes());
        out[OFF_KDF_MEMORY..OFF_KDF_MEMORY + 4].copy_from_slice(&self.kdf.memory_kib.to_be_bytes());
        out[OFF_KDF_ITERATIONS..OFF_KDF_ITERATIONS + 4]
            .copy_from_slice(&self.kdf.iterations.to_be_bytes());
        out[OFF_KDF_PARALLELISM..OFF_KDF_PARALLELISM + 4]
            .copy_from_slice(&self.kdf.parallelism.to_be_bytes());
        out[OFF_SALT..OFF_SALT + SALT_LEN].copy_from_slice(&self.salt);
        out[OFF_WRAP_NONCE..OFF_WRAP_NONCE + NONCE_LEN].copy_from_slice(&self.wrap_nonce);
        out[OFF_WRAPPED_KEY..OFF_WRAPPED_KEY + WRAPPED_KEY_LEN].copy_from_slice(&self.wrapped_key);
        out[OFF_BODY_NONCE..OFF_BODY_NONCE + NONCE_LEN].copy_from_slice(&self.body_nonce);
        out
    }

    /// Parses a whole file into the header and the body ciphertext. Never panics; the
    /// version is checked before the length so a future, longer header is reported as
    /// "newer format" rather than "unreadable".
    pub(crate) fn parse(file: &[u8]) -> Result<(Self, &[u8]), VaultError> {
        if file.len() < OFF_VERSION + 2 {
            return Err(VaultError::Unreadable("file is too short".to_owned()));
        }
        if &file[..8] != MAGIC {
            return Err(VaultError::Unreadable("not a fernjo vault file".to_owned()));
        }
        let version = u16::from_be_bytes(array(file, OFF_VERSION));
        if version > CURRENT_VERSION {
            return Err(VaultError::NewerFormat {
                found: version,
                supported: CURRENT_VERSION,
            });
        }
        if version == 0 {
            return Err(VaultError::Unreadable("invalid format version".to_owned()));
        }
        if file.len() < HEADER_LEN + TAG_LEN {
            return Err(VaultError::Unreadable("file is truncated".to_owned()));
        }
        let kdf = KdfParams {
            memory_kib: be_u32(file, OFF_KDF_MEMORY),
            iterations: be_u32(file, OFF_KDF_ITERATIONS),
            parallelism: be_u32(file, OFF_KDF_PARALLELISM),
        };
        kdf.validate()?;
        let header = Self {
            id: Uuid::from_bytes(array(file, OFF_ID)),
            kdf,
            salt: array(file, OFF_SALT),
            wrap_nonce: array(file, OFF_WRAP_NONCE),
            wrapped_key: array(file, OFF_WRAPPED_KEY),
            body_nonce: array(file, OFF_BODY_NONCE),
        };
        Ok((header, &file[HEADER_LEN..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Header {
        Header {
            id: Uuid::from_u128(7),
            kdf: KdfParams::TEST,
            salt: [1; SALT_LEN],
            wrap_nonce: [2; NONCE_LEN],
            wrapped_key: [3; WRAPPED_KEY_LEN],
            body_nonce: [4; NONCE_LEN],
        }
    }

    #[test]
    fn header_round_trips_and_has_documented_size() {
        assert_eq!(HEADER_LEN, 150);
        let h = sample();
        let mut file = h.encode().to_vec();
        file.extend_from_slice(&[9; TAG_LEN]);
        let (parsed, body) = Header::parse(&file).unwrap();
        assert_eq!(parsed.id, h.id);
        assert_eq!(parsed.kdf, h.kdf);
        assert_eq!(parsed.salt, h.salt);
        assert_eq!(parsed.wrapped_key, h.wrapped_key);
        assert_eq!(parsed.body_nonce, h.body_nonce);
        assert_eq!(body, &[9; TAG_LEN]);
    }

    #[test]
    fn bounds_accept_default_and_test_and_reject_extremes() {
        assert!(KdfParams::DEFAULT.validate().is_ok());
        assert!(KdfParams::TEST.validate().is_ok());
        let bad = |m, t, p| {
            KdfParams {
                memory_kib: m,
                iterations: t,
                parallelism: p,
            }
            .validate()
            .is_err()
        };
        assert!(bad(7, 1, 1));
        assert!(bad(8, 0, 1));
        assert!(bad(8, 21, 1));
        assert!(bad(8, 1, 0));
        assert!(bad(8 * 16, 1, 17));
        assert!(bad(15, 1, 2)); // below 8 * parallelism
        assert!(bad(MAX_MEMORY_KIB + 1, 1, 1));
    }
}
