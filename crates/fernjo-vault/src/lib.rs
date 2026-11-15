//! Encrypted credential vault for fernjo.
//!
//! No dependency on Tauri or on `fernjo-core`. The file format is documented in
//! `Docs/vault-format.md`.

mod crypto;
mod entry;
mod error;
mod format;
mod keystore;
mod vault;

pub use crypto::DataKey;
pub use entry::{EntryId, EntrySummary, EntryUpdate, NewEntry, Secret, SecretKind};
pub use error::VaultError;
pub use format::{FILE_NAME, KdfParams};
pub use keystore::{
    AutoUnlock, KeyStore, KeyStoreError, MemoryKeyStore, OsKeyStore, try_auto_unlock,
};
pub use vault::{Vault, VaultId};
