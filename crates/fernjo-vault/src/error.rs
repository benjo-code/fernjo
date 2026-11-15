//! Vault errors with stable machine-readable codes.

use thiserror::Error;

/// Everything that can go wrong in the vault. `code()` is the stable identifier the UI
/// switches on; the `Display` text is for humans and never contains secret material.
#[derive(Debug, Error)]
pub enum VaultError {
    #[error("wrong master password")]
    WrongPassword,
    #[error("the master password must have at least 8 characters")]
    WeakPassword,
    #[error("the vault is locked")]
    VaultLocked,
    #[error("entry not found")]
    NotFound,
    #[error("the vault file was modified or is corrupt")]
    Tampered,
    #[error("the vault file has format version {found}, this build understands up to {supported}")]
    NewerFormat { found: u16, supported: u16 },
    #[error("the vault file cannot be read: {0}")]
    Unreadable(String),
    #[error("the vault file asks for key derivation parameters outside the accepted range")]
    InvalidKdfParams,
    #[error("a vault file already exists")]
    AlreadyExists,
    #[error("the name must not be empty")]
    EmptyName,
    #[error("the vault file could not be written: {0}")]
    SaveFailed(String),
    #[error(
        "The vault file was changed by another device or a sync tool. Lock and unlock the vault to load the changes, then repeat your change."
    )]
    ChangedOnDisk,
}

impl VaultError {
    /// Stable code for the UI.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::WrongPassword => "wrong_password",
            Self::WeakPassword => "weak_password",
            Self::VaultLocked => "vault_locked",
            Self::NotFound => "not_found",
            Self::Tampered => "tampered",
            Self::NewerFormat { .. } => "newer_format",
            Self::Unreadable(_) => "unreadable",
            Self::InvalidKdfParams => "invalid_kdf_params",
            Self::AlreadyExists => "already_exists",
            Self::EmptyName => "empty_name",
            Self::SaveFailed(_) => "save_failed",
            Self::ChangedOnDisk => "changed_on_disk",
        }
    }
}
