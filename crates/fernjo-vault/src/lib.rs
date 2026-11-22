//! Encrypted credential vault for fernjo.
//!
//! No dependency on Tauri or on `fernjo-core`. The file format is documented in
//! `Docs/vault-format.md`.

mod crypto;
mod entry;
mod error;
mod format;
mod generation;
mod keystore;
mod sealed;
mod vault;

pub use crypto::DataKey;
pub use entry::{EntryId, EntrySummary, EntryUpdate, NewEntry, Secret, SecretKind};
pub use error::VaultError;
pub use format::{FILE_NAME, KdfParams, MAX_FILE_LEN};
pub use generation::{
    FileGenerationStore, GenerationStore, MARKS_FILE_NAME, MemoryGenerationStore,
};
pub use keystore::{
    AutoUnlock, KeyStore, KeyStoreError, MemoryKeyStore, OsKeyStore, try_auto_unlock,
};
pub use sealed::{SealedError, open_with_password, seal_with_password};
pub use vault::{Contents, FileInfo, STALE_TEMP_AGE, Vault, VaultId, inspect_file, read_entries};
