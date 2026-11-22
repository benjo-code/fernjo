//! Multi-device sync for fernjo: the snapshot of the synced data and the encrypted envelope
//! that carries it to the hub. No Tauri dependency.

pub mod envelope;
pub mod error;
pub mod snapshot;

pub use envelope::{
    FORMAT_VERSION, GroupId, MAGIC, MAX_ENVELOPE_LEN, Opened, SyncKey, open, peek_header, seal,
};
pub use error::SyncError;
pub use snapshot::{DeviceInfo, Snapshot};
pub mod client;
pub mod pairing;
