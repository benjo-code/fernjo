//! Sync errors with stable machine-readable codes. Messages never contain key material.

use std::fmt;

/// Everything that can go wrong in the sync library. `code()` is the stable identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Too short, wrong magic or larger than the limit.
    BadEnvelope,
    /// The envelope or snapshot has a newer format version than this build understands.
    NewerFormat { found: u32, supported: u32 },
    /// Authentication failed: another key, or a modified envelope.
    WrongKeyOrDamaged,
    /// The envelope belongs to another sync group.
    WrongGroup,
    /// The decrypted snapshot is not a valid snapshot.
    SnapshotInvalid(String),
    /// The snapshot is larger than an envelope may be.
    SnapshotTooLarge,
    /// The operating system gave no random bytes.
    RandomFailed,
    /// The hub did not answer (connect failure or timeout): the quiet offline state.
    HubUnreachable,
    /// The hub refused the secret (401).
    HubUnauthorized,
    /// The hub's TLS certificate is not trusted (unknown CA, wrong name, expired).
    HubTlsUntrusted,
    /// Another device wrote first (412).
    HubConflict,
    /// The hub answered something the protocol does not allow.
    HubProtocol(String),
    /// The hub's answer is larger than an envelope may be.
    HubTooLarge,
    /// The hub address is not allowed (not https, and not plain http to a loopback address).
    InvalidHubUrl(String),
    /// The pinned CA certificate is not valid PEM.
    InvalidCa,
    /// The pairing code is damaged, truncated or not a pairing code.
    InvalidPairingCode,
    /// The pairing code was made by a newer version of fernjo.
    NewerPairingCode,
    /// `sync.json` or `sync-base.json` is unreadable, damaged, too large or from a newer
    /// version; it is never overwritten.
    StateUnusable(String),
    /// A device-local sync file could not be written or removed.
    StateWriteFailed(String),
    /// The OS keychain is unavailable, locked, denied, or holds a damaged item.
    KeychainUnavailable(String),
    /// This device is not set up for sync (no `sync.json`).
    NotPaired,
    /// This device already is set up (leave first).
    AlreadyPaired,
    /// Set-up found a head on the hub: use Join with a pairing code instead.
    HubNotEmpty,
    /// Join found nothing on the hub, so there is nothing to take over.
    HubEmpty,
    /// Both sides created a vault on their own (different vault ids): not merged, nothing
    /// pushed or pulled; the owner has to decide which one stays.
    VaultConflictDifferentVaults,
    /// The local data changed between reading it and applying the merge (the port compares).
    LocalChanged,
    /// The local side (the store or the vault, behind a `LocalPort`) refused or failed; the
    /// text is the stable code of that error, never a value.
    LocalFailed(String),
    /// The hub's head is at the highest sequence number (`u64::MAX`), so nothing can be pushed
    /// above it, not even by a re-seed; only the hub's admin can clear it (remove `head.env`).
    SeqExhausted,
}

impl SyncError {
    /// The stable code the UI and tests switch on.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadEnvelope => "bad_envelope",
            Self::NewerFormat { .. } => "newer_format",
            Self::WrongKeyOrDamaged => "wrong_key_or_damaged",
            Self::WrongGroup => "wrong_group",
            Self::SnapshotInvalid(_) => "snapshot_invalid",
            Self::SnapshotTooLarge => "snapshot_too_large",
            Self::RandomFailed => "random_failed",
            Self::HubUnreachable => "hub_unreachable",
            Self::HubUnauthorized => "hub_unauthorized",
            Self::HubTlsUntrusted => "hub_tls_untrusted",
            Self::HubConflict => "hub_conflict",
            Self::HubProtocol(_) => "hub_protocol",
            Self::HubTooLarge => "hub_too_large",
            Self::InvalidHubUrl(_) => "invalid_hub_url",
            Self::InvalidCa => "invalid_ca",
            Self::InvalidPairingCode => "invalid_pairing_code",
            Self::NewerPairingCode => "newer_pairing_code",
            Self::StateUnusable(_) => "state_unusable",
            Self::StateWriteFailed(_) => "state_write_failed",
            Self::KeychainUnavailable(_) => "keychain_unavailable",
            Self::NotPaired => "not_paired",
            Self::AlreadyPaired => "already_paired",
            Self::HubNotEmpty => "hub_not_empty",
            Self::HubEmpty => "hub_empty",
            Self::VaultConflictDifferentVaults => "vault_conflict_different_vaults",
            Self::LocalChanged => "local_changed",
            Self::LocalFailed(_) => "local_failed",
            Self::SeqExhausted => "seq_exhausted",
        }
    }
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadEnvelope => f.write_str("not a sync envelope"),
            Self::NewerFormat { found, supported } => write!(
                f,
                "format version {found} is newer than the supported version {supported}"
            ),
            Self::WrongKeyOrDamaged => f.write_str("wrong sync key, or the data is damaged"),
            Self::WrongGroup => f.write_str("the data belongs to another sync group"),
            Self::SnapshotInvalid(reason) => write!(f, "the snapshot is invalid: {reason}"),
            Self::SnapshotTooLarge => f.write_str("the snapshot is too large"),
            Self::RandomFailed => f.write_str("no random source available"),
            Self::HubUnreachable => f.write_str("the sync hub is not reachable"),
            Self::HubUnauthorized => f.write_str("the sync hub refused the secret"),
            Self::HubTlsUntrusted => f.write_str("the sync hub's certificate is not trusted"),
            Self::HubConflict => f.write_str("another device changed the data first"),
            Self::HubProtocol(reason) => write!(f, "unexpected answer from the sync hub: {reason}"),
            Self::HubTooLarge => f.write_str("the sync hub's answer is too large"),
            Self::InvalidHubUrl(reason) => write!(f, "the hub address is not allowed: {reason}"),
            Self::InvalidCa => f.write_str("the CA certificate is not valid PEM"),
            Self::InvalidPairingCode => f.write_str("this is not a valid pairing code"),
            Self::NewerPairingCode => {
                f.write_str("this pairing code was made by a newer version of fernjo")
            }
            Self::StateUnusable(reason) => {
                write!(f, "the device-local sync data is unusable: {reason}")
            }
            Self::StateWriteFailed(reason) => {
                write!(f, "the device-local sync data could not be saved: {reason}")
            }
            Self::KeychainUnavailable(reason) => write!(f, "OS keychain: {reason}"),
            Self::NotPaired => f.write_str("this device is not set up for sync"),
            Self::AlreadyPaired => f.write_str("this device is already set up for sync"),
            Self::HubNotEmpty => {
                f.write_str("the hub already holds data: join with a pairing code instead")
            }
            Self::HubEmpty => f.write_str("the hub holds no data to join"),
            Self::VaultConflictDifferentVaults => f.write_str(
                "this device and the hub each created their own vault; they cannot be merged",
            ),
            Self::LocalChanged => f.write_str("the local data changed while syncing"),
            Self::LocalFailed(code) => write!(f, "the local data could not be updated ({code})"),
            Self::SeqExhausted => f.write_str(
                "the hub's sequence number is at its maximum; nothing can be pushed until the hub's admin removes its head",
            ),
        }
    }
}

impl std::error::Error for SyncError {}
