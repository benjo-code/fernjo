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
        }
    }
}

impl std::error::Error for SyncError {}
