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
}

impl StoreError {
    /// Stable code shown to the UI.
    pub fn code(&self) -> &'static str {
        match self {
            StoreError::Tree(e) => e.code(),
            StoreError::Save(_) => "save_failed",
        }
    }
}
