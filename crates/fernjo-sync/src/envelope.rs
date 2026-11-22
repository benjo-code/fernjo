//! The sealed snapshot: XChaCha20-Poly1305 under a key derived from the sync key with HKDF-SHA256.
//!
//! Layout, all integers big endian:
//!
//! ```text
//! offset  size  field
//!      0     8  magic "FERNJOSY"
//!      8     2  format version, u16
//!     10    16  group id
//!     26     8  seq, u64
//!     34    24  nonce
//!     58     n  ciphertext + 16 byte tag
//! ```
//!
//! Bytes 0..34 (magic, version, group, seq) are the associated data, so the hub can read
//! the head without the key but nobody can move a ciphertext to another seq or group.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::error::SyncError;

/// Identifies one sync group (one set of devices sharing one hub and key).
pub type GroupId = Uuid;

pub const MAGIC: &[u8; 8] = b"FERNJOSY";
pub const FORMAT_VERSION: u16 = 1;
pub const MAX_ENVELOPE_LEN: usize = 64 * 1024 * 1024;

const OFF_VERSION: usize = 8;
const OFF_GROUP: usize = 10;
const OFF_SEQ: usize = 26;
const OFF_NONCE: usize = 34;
const AAD_LEN: usize = OFF_NONCE;
const NONCE_LEN: usize = 24;
const HEADER_LEN: usize = OFF_NONCE + NONCE_LEN;
const TAG_LEN: usize = 16;
const HKDF_INFO: &[u8] = b"fernjo-sync v1 snapshot";

/// The random 256-bit key shared by the devices of a group. Zeroised on drop, never printed.
pub struct SyncKey([u8; 32]);

impl SyncKey {
    /// A fresh random key.
    pub fn generate() -> Result<SyncKey, SyncError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|_| SyncError::RandomFailed)?;
        Ok(SyncKey(bytes))
    }

    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> SyncKey {
        SyncKey(bytes)
    }

    /// The raw key bytes, for the keychain and the pairing code only.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for SyncKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyncKey(<redacted>)")
    }
}

/// A decrypted envelope.
pub struct Opened {
    pub group: GroupId,
    pub seq: u64,
    pub plaintext: Zeroizing<Vec<u8>>,
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("group", &self.group)
            .field("seq", &self.seq)
            .field("plaintext", &"<redacted>")
            .finish()
    }
}

fn cipher(key: &SyncKey) -> XChaCha20Poly1305 {
    let hk = Hkdf::<Sha256>::new(None, key.as_bytes());
    let mut okm = Zeroizing::new([0u8; 32]);
    hk.expand(HKDF_INFO, okm.as_mut())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    XChaCha20Poly1305::new((&*okm).into())
}

/// Encrypts `plaintext` as envelope number `seq` of `group`.
pub fn seal(
    key: &SyncKey,
    group: GroupId,
    seq: u64,
    plaintext: &[u8],
) -> Result<Vec<u8>, SyncError> {
    if plaintext.len().saturating_add(HEADER_LEN + TAG_LEN) > MAX_ENVELOPE_LEN {
        return Err(SyncError::SnapshotTooLarge);
    }
    let mut out = Vec::with_capacity(HEADER_LEN + plaintext.len() + TAG_LEN);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    out.extend_from_slice(group.as_bytes());
    out.extend_from_slice(&seq.to_be_bytes());
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| SyncError::RandomFailed)?;
    out.extend_from_slice(&nonce);
    let ciphertext = cipher(key)
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: &out[..AAD_LEN],
            },
        )
        .map_err(|_| SyncError::WrongKeyOrDamaged)?;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Reads group id and seq without authenticating them (for the hub and for rollback checks
/// before any decryption). Does not check the key.
pub fn peek_header(bytes: &[u8]) -> Result<(GroupId, u64), SyncError> {
    if bytes.len() > MAX_ENVELOPE_LEN || bytes.len() < HEADER_LEN + TAG_LEN {
        return Err(SyncError::BadEnvelope);
    }
    if &bytes[..8] != MAGIC {
        return Err(SyncError::BadEnvelope);
    }
    let version = u16::from_be_bytes([bytes[OFF_VERSION], bytes[OFF_VERSION + 1]]);
    if version > FORMAT_VERSION {
        return Err(SyncError::NewerFormat {
            found: u32::from(version),
            supported: u32::from(FORMAT_VERSION),
        });
    }
    if version == 0 {
        return Err(SyncError::BadEnvelope);
    }
    let group = Uuid::from_bytes(
        bytes[OFF_GROUP..OFF_SEQ]
            .try_into()
            .expect("slice length is 16"),
    );
    let seq = u64::from_be_bytes(
        bytes[OFF_SEQ..OFF_NONCE]
            .try_into()
            .expect("slice length is 8"),
    );
    Ok((group, seq))
}

/// Authenticates and decrypts an envelope.
pub fn open(key: &SyncKey, bytes: &[u8]) -> Result<Opened, SyncError> {
    let (group, seq) = peek_header(bytes)?;
    let nonce: [u8; NONCE_LEN] = bytes[OFF_NONCE..HEADER_LEN]
        .try_into()
        .expect("slice length is 24");
    let plaintext = cipher(key)
        .decrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &bytes[HEADER_LEN..],
                aad: &bytes[..AAD_LEN],
            },
        )
        .map_err(|_| SyncError::WrongKeyOrDamaged)?;
    Ok(Opened {
        group,
        seq,
        plaintext: Zeroizing::new(plaintext),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SyncKey {
        SyncKey::from_bytes([7u8; 32])
    }

    fn group() -> GroupId {
        Uuid::from_u128(0x1234)
    }

    #[test]
    fn seal_then_open_round_trips() {
        let env = seal(&key(), group(), 42, b"hello").unwrap();
        assert_eq!(peek_header(&env).unwrap(), (group(), 42));
        let opened = open(&key(), &env).unwrap();
        assert_eq!(opened.group, group());
        assert_eq!(opened.seq, 42);
        assert_eq!(opened.plaintext.as_slice(), b"hello");
    }

    #[test]
    fn open_with_another_key_fails_as_wrong_key_or_damaged() {
        let env = seal(&key(), group(), 1, b"x").unwrap();
        let err = open(&SyncKey::from_bytes([8u8; 32]), &env).unwrap_err();
        assert_eq!(err.code(), "wrong_key_or_damaged");
    }

    #[test]
    fn flipping_any_header_byte_fails_authentication() {
        let env = seal(&key(), group(), 5, b"payload").unwrap();
        for i in 0..AAD_LEN {
            let mut bad = env.clone();
            bad[i] ^= 0xFF;
            let err = open(&key(), &bad)
                .err()
                .unwrap_or_else(|| panic!("byte {i} flipped and the envelope still opened"));
            assert!(
                ["wrong_key_or_damaged", "bad_envelope", "newer_format"].contains(&err.code()),
                "byte {i}: {}",
                err.code()
            );
        }
        // the nonce is covered by the cipher itself
        let mut bad = env.clone();
        bad[OFF_NONCE] ^= 1;
        assert_eq!(
            open(&key(), &bad).unwrap_err().code(),
            "wrong_key_or_damaged"
        );
    }

    #[test]
    fn flipping_a_ciphertext_byte_fails() {
        let env = seal(&key(), group(), 1, b"payload").unwrap();
        for i in HEADER_LEN..env.len() {
            let mut bad = env.clone();
            bad[i] ^= 1;
            assert_eq!(
                open(&key(), &bad).unwrap_err().code(),
                "wrong_key_or_damaged"
            );
        }
    }

    #[test]
    fn a_head_moved_to_another_seq_fails() {
        let mut env = seal(&key(), group(), 1, b"payload").unwrap();
        env[OFF_SEQ..OFF_NONCE].copy_from_slice(&2u64.to_be_bytes());
        assert_eq!(peek_header(&env).unwrap().1, 2);
        assert_eq!(
            open(&key(), &env).unwrap_err().code(),
            "wrong_key_or_damaged"
        );
    }

    #[test]
    fn newer_version_is_refused_as_newer_format() {
        let mut env = seal(&key(), group(), 1, b"payload").unwrap();
        env[OFF_VERSION..OFF_VERSION + 2].copy_from_slice(&2u16.to_be_bytes());
        assert_eq!(open(&key(), &env).unwrap_err().code(), "newer_format");
        assert_eq!(peek_header(&env).unwrap_err().code(), "newer_format");
    }

    #[test]
    fn short_or_wrong_magic_is_bad_envelope() {
        let env = seal(&key(), group(), 1, b"payload").unwrap();
        assert_eq!(open(&key(), &env[..10]).unwrap_err().code(), "bad_envelope");
        assert_eq!(open(&key(), &[]).unwrap_err().code(), "bad_envelope");
        let mut bad = env.clone();
        bad[0] = b'X';
        assert_eq!(open(&key(), &bad).unwrap_err().code(), "bad_envelope");
        assert_eq!(peek_header(&bad).unwrap_err().code(), "bad_envelope");
    }

    #[test]
    fn nonce_is_fresh_per_seal() {
        let a = seal(&key(), group(), 1, b"same").unwrap();
        let b = seal(&key(), group(), 1, b"same").unwrap();
        assert_ne!(a, b);
        assert_ne!(a[OFF_NONCE..HEADER_LEN], b[OFF_NONCE..HEADER_LEN]);
    }

    #[test]
    fn a_envelope_above_64_mib_is_refused_before_decryption() {
        let mut big = seal(&key(), group(), 1, b"x").unwrap();
        big.resize(MAX_ENVELOPE_LEN + 1, 0);
        assert_eq!(open(&key(), &big).unwrap_err().code(), "bad_envelope");
        assert_eq!(peek_header(&big).unwrap_err().code(), "bad_envelope");
    }

    #[test]
    fn a_plaintext_that_cannot_fit_is_snapshot_too_large() {
        let big = vec![0u8; MAX_ENVELOPE_LEN];
        assert_eq!(
            seal(&key(), group(), 1, &big).unwrap_err().code(),
            "snapshot_too_large"
        );
    }

    #[test]
    fn debug_of_sync_key_is_redacted() {
        let k = SyncKey::from_bytes([0xAB; 32]);
        let text = format!("{k:?}");
        assert!(text.contains("redacted"));
        assert!(!text.contains("171"));
        let opened = Opened {
            group: group(),
            seq: 1,
            plaintext: Zeroizing::new(b"secretdata".to_vec()),
        };
        assert!(!format!("{opened:?}").contains("secretdata"));
    }

    #[test]
    fn generated_keys_differ() {
        let a = SyncKey::generate().unwrap();
        let b = SyncKey::generate().unwrap();
        assert_ne!(a.as_bytes(), b.as_bytes());
    }
}
