//! Error types of the core crate.

use crate::model::NodeId;

/// A rule of the tree was violated. Every variant has a stable machine-readable code.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    #[error("no such item: {0}")]
    NotFound(NodeId),
    #[error("the target is not a folder")]
    NotAFolder,
    #[error("the item is not a connection")]
    NotAConnection,
    #[error("the name must not be empty")]
    EmptyName,
    #[error("the port must be between 1 and 65535")]
    InvalidPort,
    #[error("a folder cannot be moved into itself or one of its subfolders")]
    InvalidMove,
    #[error("the position is outside the folder")]
    IndexOutOfRange,
    #[error("an item with the id {0} already exists")]
    DuplicateId(NodeId),
    #[error(
        "folders cannot be nested more than {} levels deep",
        crate::tree::MAX_TREE_DEPTH
    )]
    TooDeep,
}

impl TreeError {
    /// Stable code shown to the UI and used in tests.
    pub fn code(&self) -> &'static str {
        match self {
            TreeError::NotFound(_) => "not_found",
            TreeError::NotAFolder => "not_a_folder",
            TreeError::NotAConnection => "not_a_connection",
            TreeError::EmptyName => "empty_name",
            TreeError::InvalidPort => "invalid_port",
            TreeError::InvalidMove => "invalid_move",
            TreeError::IndexOutOfRange => "index_out_of_range",
            TreeError::DuplicateId(_) => "duplicate_id",
            TreeError::TooDeep => "too_deep",
        }
    }
}

/// `connections.json` could not be written.
#[derive(Debug, thiserror::Error)]
#[error("could not write {}: {source}", path.display())]
pub struct SaveError {
    pub path: std::path::PathBuf,
    #[source]
    pub source: std::io::Error,
}

/// `connections.json` could not be loaded. Nothing has been overwritten in either case.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(
        "{} was written by a newer version of fernjo (file format {found}, this version understands {supported})",
        path.display()
    )]
    NewerSchema {
        found: u32,
        supported: u32,
        path: std::path::PathBuf,
    },
    #[error("could not read {}: {reason}", path.display())]
    Unreadable {
        path: std::path::PathBuf,
        reason: String,
    },
}

/// A store mutation failed; memory and disk are unchanged.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error(transparent)]
    Save(#[from] SaveError),
    #[error(
        "connections.json was changed by another program or device since fernjo last read it; reload to see those changes"
    )]
    ChangedOnDisk,
}

impl StoreError {
    /// Stable code shown to the UI.
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::Tree(e) => e.code(),
            StoreError::Save(_) => "save_failed",
            StoreError::ChangedOnDisk => "changed_on_disk",
        }
    }
}

/// A `connections.json` document (in memory, not read from a file) could not be turned
/// into a tree.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DocumentError {
    #[error(
        "the document was written by a newer version of fernjo (file format {found}, this version understands {supported})"
    )]
    NewerSchema { found: u32, supported: u32 },
    #[error("invalid document: {0}")]
    Invalid(String),
}

impl DocumentError {
    /// Stable code shown to the UI and used in tests.
    pub fn code(&self) -> &'static str {
        match self {
            DocumentError::NewerSchema { .. } => "newer_schema",
            DocumentError::Invalid(_) => "invalid_document",
        }
    }
}
