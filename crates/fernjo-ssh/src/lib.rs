//! SSH client sessions for fernjo. No dependency on Tauri, `fernjo-core` or `fernjo-vault`.

mod error;
mod known_hosts;

pub use error::SshError;
pub use known_hosts::{HostKeyStatus, KnownHosts, fingerprint};
