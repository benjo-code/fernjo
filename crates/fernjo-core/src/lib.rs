//! Core model of fernjo: connection tree, inheritance and persistence.

pub mod conflicts;
pub mod error;
pub mod model;
pub mod paths;
pub mod persist;
pub mod resolve;
pub mod settings;
pub mod store;
pub mod tree;

pub use conflicts::find_conflicts;
pub use error::{DocumentError, LoadError, SaveError, StoreError, TreeError};
pub use model::{
    CredentialId, DEFAULT_KEEPALIVE_SECS, DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, Field, Node, NodeId,
    NodeKind, NodeType, Protocol, ResolvedNode, Settings, Source,
};
pub use paths::{DATA_DIR_ENV, DataDirSource, Location, locate, locate_from};
pub use persist::{
    CURRENT_VERSION, FILE_NAME, Loaded, Notice, document, load, migrate_with, parse_document, save,
    write_exact,
};
pub use resolve::resolve;
pub use settings::{
    AppSettings, DEFAULT_FONT_SIZE, DEFAULT_SCROLLBACK, FONT_SIZE_RANGE, SCROLLBACK_RANGE,
    TerminalPrefs, Theme,
};
pub use store::{Opened, Store};
pub use tree::{MAX_TREE_DEPTH, NodePatch, Patch, Tree, subtree_depth};

/// Returns the crate version.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        let parts: Vec<&str> = version().split('.').collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.parse::<u32>().is_ok()));
    }
}
