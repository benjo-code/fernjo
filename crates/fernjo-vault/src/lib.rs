//! Encrypted credential vault for fernjo.
//!
//! No dependency on Tauri or on `fernjo-core`. The file format is documented in
//! `Docs/vault-format.md`.

mod crypto;
mod error;
mod format;
mod vault;

pub use crypto::DataKey;
pub use error::VaultError;
pub use format::{FILE_NAME, KdfParams};
pub use vault::{Vault, VaultId};
