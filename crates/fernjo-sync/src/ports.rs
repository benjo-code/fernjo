//! What the engine needs from the app: read the synced local data and replace it. The Tauri
//! side implements [`LocalPort`] over `Store` and `Vault` (taking its locks inside `read` and
//! `apply`); the engine itself holds no lock and knows no `AppState`.

use std::fmt;

use fernjo_core::Tree;
use fernjo_vault::{DataKey, EntryId, NewEntry};

use crate::error::SyncError;

/// The local vault as sync sees it. `entries` and `key` are present only while it is unlocked.
pub struct VaultSide {
    /// The `vault.fernjo` bytes as the open vault last read or wrote them (`Vault::file_bytes`
    /// refuses a file that changed on disk behind it).
    pub file: Vec<u8>,
    pub entries: Option<Vec<(EntryId, NewEntry)>>,
    pub key: Option<DataKey>,
}

impl fmt::Debug for VaultSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultSide")
            .field("file_len", &self.file.len())
            .field("unlocked", &self.key.is_some())
            .finish()
    }
}

/// The synced local data at one moment.
pub struct LocalState {
    pub tree: Tree,
    pub vault: Option<VaultSide>,
}

impl fmt::Debug for LocalState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalState")
            .field("vault", &self.vault)
            .finish_non_exhaustive()
    }
}

/// Two states are equal when the tree and the vault file bytes are: that is what "local data is
/// still what was merged" means. The decrypted entries and the key follow from the file.
impl PartialEq for LocalState {
    fn eq(&self, other: &Self) -> bool {
        self.tree == other.tree
            && self.vault.as_ref().map(|v| &v.file) == other.vault.as_ref().map(|v| &v.file)
    }
}

/// What to do to the local vault.
pub enum VaultChange {
    /// Leave it as it is.
    Keep,
    /// Make these bytes the vault file as they are (only the other side changed it); works
    /// while the vault is locked. The bytes can belong to another vault than the local one (a
    /// device that created its own vault and then joined a group whose head has one);
    /// `Vault::adopt_file` refuses another vault id, so the port must then replace the vault
    /// file itself.
    Adopt(Vec<u8>),
    /// Both sides changed it: write `entries` with the header of `head` and a generation above
    /// both (needs the vault unlocked). `base_generation` is the generation of the base file (0
    /// without one); `Vault::adopt_merged` refuses a head below it (`rolled_back`; a port reports
    /// such a refusal through `engine::refused_head`, as it does for `adopt_file`).
    Merged {
        head: Vec<u8>,
        base_generation: u64,
        entries: Vec<(EntryId, NewEntry)>,
    },
}

impl fmt::Debug for VaultChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keep => f.write_str("Keep"),
            Self::Adopt(bytes) => write!(f, "Adopt({} bytes)", bytes.len()),
            Self::Merged { head, entries, .. } => {
                write!(f, "Merged({} bytes, {} entries)", head.len(), entries.len())
            }
        }
    }
}

/// The change the engine asks the port to make in one step.
#[derive(Debug)]
pub struct LocalChange {
    /// The tree to store; `None` leaves it.
    pub tree: Option<Tree>,
    pub vault: VaultChange,
}

pub trait LocalPort {
    /// The current local data (locks are taken and released inside).
    ///
    /// # Errors
    /// `local_failed` when it cannot be read.
    fn read(&self) -> Result<LocalState, SyncError>;

    /// Applies `next` if the local data still equals `expected` (the port compares, with
    /// `LocalState`'s equality) and refuses with `local_changed` otherwise. Goes through
    /// `Store` and `Vault`, so their guards run. Nothing is written on an error.
    ///
    /// # Errors
    /// `local_changed`, `local_failed`.
    fn apply(&self, expected: &LocalState, next: LocalChange) -> Result<(), SyncError>;

    /// Whether the vault is unlocked right now.
    fn vault_unlocked(&self) -> bool;
}
