//! Core model of fernjo: connection tree, inheritance and persistence.

pub mod model;
pub mod resolve;

pub use model::{
    DEFAULT_KEEPALIVE_SECS, DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, Field, Node, NodeId, NodeKind,
    NodeType, Protocol, ResolvedNode, Settings, Source,
};
pub use resolve::resolve;

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
