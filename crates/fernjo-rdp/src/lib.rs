//! RDP client sessions for fernjo. No dependency on Tauri, `fernjo-core` or `fernjo-vault`.

mod cert_store;
mod clipboard;
mod error;
mod framebuffer;
mod input;
mod interaction;

pub use cert_store::{CertCheck, CertStore};
pub use clipboard::{MAX_CLIPBOARD_BYTES, from_remote, to_remote};
pub use error::RdpError;
pub use framebuffer::{Framebuffer, MAX_DESKTOP_DIMENSION, Rect};
pub use input::{InputEvent, MAX_INPUT_EVENTS, MouseButton};
pub use interaction::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, Interaction,
};
