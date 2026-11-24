//! RDP client sessions for fernjo. No dependency on Tauri, `fernjo-core` or `fernjo-vault`.

mod cert_store;
mod clipboard;
mod connect;
mod error;
mod framebuffer;
mod input;
mod interaction;
mod params;
mod session;
mod text_clipboard;
mod tls;

pub use cert_store::{CertCheck, CertStore};
pub use clipboard::{MAX_CLIPBOARD_BYTES, from_remote, to_remote};
pub use error::RdpError;
pub use framebuffer::{Framebuffer, MAX_DESKTOP_DIMENSION, MAX_DESKTOP_PIXELS, Rect};
pub use input::{InputEvent, MAX_INPUT_EVENTS, MouseButton};
pub use interaction::{
    CertChangedDecision, CertChangedPrompt, CertDecision, CertPrompt, Interaction,
};
pub use params::ConnectParams;
pub use session::{CloseReason, Session, SessionEvent};
