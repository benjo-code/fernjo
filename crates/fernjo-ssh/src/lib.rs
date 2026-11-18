//! SSH client sessions for fernjo. No dependency on Tauri, `fernjo-core` or `fernjo-vault`.

mod agent;
mod auth_plan;
mod error;
mod handler;
mod interaction;
mod known_hosts;
mod params;
mod session;

pub use auth_plan::{Attempt, AuthPlanner, Available, MAX_TOTAL, Offered};
pub use error::SshError;
pub use interaction::{
    HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt,
    HostKeySyncedPrompt, Interaction, KiPrompt, KiRound,
};
pub use known_hosts::{HostKeyStatus, KnownHosts, fingerprint, validate_host};
pub use params::{ConnectParams, Credential};
pub use session::{CloseReason, Session, SessionEvent};
