//! The fernjo sync hub: one encrypted snapshot head with compare-and-swap, behind a bearer secret.
//!
//! The hub never depends on `fernjo-sync` and never holds the sync key; it reads only the 34
//! unauthenticated header bytes of an envelope (magic, version, group id, `seq`).

pub mod config;
pub mod server;
pub mod store;

pub use server::{router, serve, shutdown_signal};
pub use store::{Expected, GroupId, HubStore, MAX_ENVELOPE_LEN, PutError, peek_header};
