//! The session manager: terminal and RDP sessions, their prompts and their output flow.
//!
//! # Wire contract (what the UI mirrors)
//!
//! One `tauri::ipc::Channel<InvokeResponseBody>` per session, created by the webview and passed to
//! `open_session`. Two kinds of messages travel on it, in order (Tauri's JS `Channel` re-orders
//! by message index):
//!
//! * **Terminal output** is `InvokeResponseBody::Raw`: the JS `onmessage` handler receives an
//!   `ArrayBuffer` (batches of about 16 ms or at most [`MAX_BATCH`] bytes).
//! * **Events** are `InvokeResponseBody::Json`: the handler receives the parsed object
//!   ([`UiEvent`]): `{kind:"state", state, detail, exitStatus, endedCleanly}` or
//!   `{kind:"prompt", id, prompt}`. `exitStatus` (the remote shell's exit code, else `null`) and
//!   `endedCleanly` (`true` only for exit status 0 after a session that lasted at least
//!   [`MIN_AUTO_CLOSE_SESSION`]) are set on the `disconnected` event that ends a connected
//!   session and are `null`/`false` on every other event; the UI decides what to do with them.
//!
//! **Restored tabs:** `restore_session` (same `on_event` channel, `cols` and `rows` as
//! `open_session`) creates the session without connecting. Its only event is `{kind:"state",
//! state:"disconnected", detail:RESTORED_TEXT, exitStatus:null, endedCleanly:false}`; the tab
//! then behaves like any disconnected one, and `reconnect_session` resolves the plan from the
//! tree as it is then. A restored session is never "ended cleanly" and does not count as
//! connected for the close confirmation.
//!
//! The UI tells them apart with `message instanceof ArrayBuffer`. Input travels the other way as
//! the raw request body of `session_input` (the session id in the `x-fernjo-session` header).
//! The UI acknowledges output with `session_ack(id, bytes)` once xterm.js has written it; more
//! than `session_flow::HIGH_WATER` unacknowledged bytes pause the reader until the count falls
//! to `session_flow::LOW_WATER` (see `session_flow`).
//!
//! Output in flight between the server and xterm.js is bounded on purpose, because everything
//! ahead of the echo of Ctrl+C has to drain through xterm's parser: SSH window 512 KiB, 4 packets
//! buffered in the russh channel, 4 events in `fernjo_ssh`'s queue (32 KiB packets), `HIGH_WATER`
//! 128 KiB unacknowledged and at most one more batch (`MAX_BATCH` 32 KiB). Details and
//! measurements: `Docs/DECISIONS.md`, "M4 flow control numbers".
//!
//! Host keys: `hostKey` is a host that is not in `known_hosts`, `hostKeyChanged` a stored key that
//! differs, and `hostKeySynced` a key that `known_hosts` has but this device has not confirmed
//! (another device or a file added it; it carries `previous`, the fingerprints this device
//! confirmed before for that host, port and key type). `hostKey` and `hostKeySynced` are answered
//! with `{kind:"hostKey", decision:"once"|"always"|"reject"}`; for the synced one "always" confirms
//! the key on this device and "once" records nothing.
//!
//! # RDP sessions
//!
//! An RDP connection uses the same channel, events, prompts and commands with these differences
//! (the code is in `sessions/rdp.rs`):
//!
//! * The raw message is a **region** of the desktop: `u16 x, u16 y, u16 width, u16 height` (little
//!   endian) followed by `width * height * 4` RGBA bytes, row by row. The forwarder sends one
//!   region per available credit, covering everything that changed since the last one (the latest
//!   pixels win). `session_ack(id, bytes)` acknowledges a region by its byte length; at most about
//!   `rdp::RDP_FRAMES_IN_FLIGHT` (3) whole-desktop messages are unacknowledged (the number comes
//!   from the spike, `Docs/DECISIONS.md` "M8: RDP render path spike").
//! * Two more JSON events: `{kind:"desktop", width, height}` (before the first region and on every
//!   desktop size change) and `{kind:"clipboard", text}` (the remote side copied text).
//! * `open_session`, `session_resize` and `reconnect_session` carry **pixels** in their `cols` and
//!   `rows` arguments. Input is `rdp_input(id, events)` (not `session_input`) and clipboard text
//!   `rdp_clipboard(id, text)`.
//! * Prompts: `certificate {host, port, fingerprint}` is answered with `{kind:"hostKey",
//!   decision}` and `certificateChanged {host, port, old, new}` with `{kind:"hostKeyChanged",
//!   decision}`; `username` and `password` are the shared ones. Pins live in
//!   `<device folder>/rdp_certs.confirmed` (memory for the run without a device folder).
//! * A failed connect ends with the text of the `fernjo_rdp::RdpError`. A server whose
//!   certificate rustls cannot use (webpki cannot parse it, or its key cannot verify the handshake
//!   signature, for example RSA under 2048 bits) fails before any prompt with "the server
//!   certificate is not supported" (`certificate_unsupported`); other TLS failures keep "the secure
//!   connection to the server failed" (`tls_failed`).
//! * An RDP session never ends "cleanly" (`endedCleanly` is always `false`): there is no shell exit
//!   status and the tab stays until the user closes it. A server that announces a virtual channel
//!   message above 4 MiB ends the session with the detail "The server sent a channel message that
//!   is too large." (`fernjo_rdp::CloseReason::ChannelTooLarge`).
//!
//! No event carries a secret: prompts describe what is asked, answers travel UI to Rust only.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fernjo_core::NodeId;
use fernjo_ssh::{
    CloseReason, ConnectParams, HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision,
    HostKeyPrompt, HostKeySyncedPrompt, Interaction, KiRound, KnownHosts, Session, SessionEvent,
};
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize};
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc::{Receiver, UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;

use crate::commands::CommandError;
use crate::connect_plan::{self, ConnectPlan, CredentialPlan, Plan, Target};
use crate::session_flow::{FlowControl, InputBudget, MIN_OP_WEIGHT};
use crate::state::AppState;

/// Largest raw output message.
pub const MAX_BATCH: usize = 32 * 1024;
/// How long output is collected before it is sent.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(16);
/// A session that ended with exit status 0 counts as a deliberate, clean end only after it was
/// connected at least this long, so a shell that dies at once (a broken profile script) keeps its
/// tab and its message.
pub const MIN_AUTO_CLOSE_SESSION: Duration = Duration::from_secs(5);
/// The reason text of a tab restored from the last run: it exists, but nothing is connected.
pub const RESTORED_TEXT: &str = "Restored from your last session. Not connected.";

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.into(),
    }
}

fn not_found() -> CommandError {
    error("not_found", "there is no such session")
}

/// Locks, mapping a poisoned lock to the generic internal error.
fn lock<T>(m: &Mutex<T>) -> Result<std::sync::MutexGuard<'_, T>, CommandError> {
    m.lock().map_err(|_| CommandError::internal())
}

// ---------------------------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnState {
    Connecting,
    NeedsVault,
    Connected,
    Disconnected,
}

/// A JSON message on the session channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum UiEvent {
    State {
        state: ConnState,
        detail: Option<String>,
        /// The remote shell's exit code; set only when a connected session ended with one.
        exit_status: Option<u32>,
        /// See [`classify_end`]; `false` for every state except a clean end.
        ended_cleanly: bool,
    },
    Prompt {
        id: u64,
        prompt: PromptView,
    },
    /// RDP only: the remote desktop has this size (pixels). Sent before the first region and on
    /// every size change; the canvas takes its size from it.
    Desktop {
        width: u16,
        height: u16,
    },
    /// RDP only: the remote side copied this text (line ends converted, size capped).
    Clipboard {
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KiPromptView {
    pub text: String,
    pub echo: bool,
}

/// What the user is asked. Never contains a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PromptView {
    HostKey {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
    },
    HostKeySynced {
        host: String,
        port: u16,
        key_type: String,
        fingerprint: String,
        previous: Vec<String>,
    },
    HostKeyChanged {
        host: String,
        port: u16,
        key_type: String,
        old: Vec<String>,
        new: String,
    },
    KeyboardInteractive {
        name: String,
        instructions: String,
        prompts: Vec<KiPromptView>,
    },
    /// RDP: a server certificate this device has not pinned for `host:port`. Answered with
    /// `{kind:"hostKey", decision}` (the same shape as an SSH host key).
    Certificate {
        host: String,
        port: u16,
        fingerprint: String,
    },
    /// RDP: the certificate differs from the pinned one. Answered with
    /// `{kind:"hostKeyChanged", decision}`.
    CertificateChanged {
        host: String,
        port: u16,
        old: String,
        new: String,
    },
    Password {
        user: String,
        host: String,
    },
    Username {
        host: String,
    },
    KeyPassphrase {},
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKeyAnswer {
    Once,
    Always,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostKeyChangedAnswer {
    Replace,
    Reject,
}

fn secret_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<SecretString>, D::Error> {
    Ok(Vec::<String>::deserialize(d)?
        .into_iter()
        .map(SecretString::from)
        .collect())
}

/// The user's answer to a prompt (UI to Rust only). `Debug` shows no typed text.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PromptAnswer {
    HostKey {
        decision: HostKeyAnswer,
    },
    HostKeyChanged {
        decision: HostKeyChangedAnswer,
    },
    /// Passwords, passphrases and one-time codes.
    Secrets {
        #[serde(deserialize_with = "secret_list")]
        values: Vec<SecretString>,
    },
    /// Plain text (the user name).
    Text {
        value: String,
    },
    Cancel {},
}

impl fmt::Debug for PromptAnswer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostKey { decision } => write!(f, "PromptAnswer::HostKey({decision:?})"),
            Self::HostKeyChanged { decision } => {
                write!(f, "PromptAnswer::HostKeyChanged({decision:?})")
            }
            Self::Secrets { values } => {
                write!(f, "PromptAnswer::Secrets(<{} redacted>)", values.len())
            }
            Self::Text { .. } => f.write_str("PromptAnswer::Text(<redacted>)"),
            Self::Cancel {} => f.write_str("PromptAnswer::Cancel"),
        }
    }
}

/// Accepts only absolute `http` and `https` URLs with a non-empty host and returns the parsed
/// URL. Callers must open `parsed.as_str()`, never the input: the parser strips surrounding
/// whitespace and embedded tabs and newlines and normalises `http:/x` to `http://x/`, so the
/// input text is not what was validated.
///
/// # Errors
/// `invalid_url` for anything else.
pub fn check_external_url(url: &str) -> Result<tauri::Url, CommandError> {
    let refused = || error("invalid_url", "only http and https links can be opened");
    let parsed = tauri::Url::parse(url).map_err(|_| refused())?;
    let has_host = parsed.host_str().is_some_and(|host| !host.is_empty());
    if matches!(parsed.scheme(), "http" | "https") && has_host {
        Ok(parsed)
    } else {
        Err(refused())
    }
}

// ---------------------------------------------------------------------------------------------
// Prompt hub
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct HubInner {
    closed: bool,
    waiting: HashMap<u64, oneshot::Sender<PromptAnswer>>,
}

/// The prompts one session has open. Dropping the hub (or [`PromptHub::close`]) drops every
/// waiting sender, so every waiting ask resolves with the safe answer.
#[derive(Default)]
pub struct PromptHub {
    next: AtomicU64,
    inner: Mutex<HubInner>,
}

impl PromptHub {
    /// Registers a prompt; `None` once the hub is closed. Register before showing the prompt so
    /// an answer can never arrive first.
    pub fn register(&self) -> Option<(u64, oneshot::Receiver<PromptAnswer>)> {
        let mut inner = self.inner.lock().ok()?;
        if inner.closed {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        inner.waiting.insert(id, tx);
        Some((id, rx))
    }

    /// Delivers an answer to the waiting ask.
    ///
    /// # Errors
    /// `not_found` if no such prompt is waiting (unknown, already answered or closed).
    pub fn answer(&self, id: u64, answer: PromptAnswer) -> Result<(), CommandError> {
        let sender = lock(&self.inner)?.waiting.remove(&id);
        let sender = sender.ok_or_else(|| error("not_found", "there is no such prompt"))?;
        // A receiver that is gone (the connect ended meanwhile) is not an error for the UI.
        let _ = sender.send(answer);
        Ok(())
    }

    /// Resolves every waiting ask with the safe answer and refuses new ones.
    pub fn close(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.closed = true;
            inner.waiting.clear();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Sink and interaction
// ---------------------------------------------------------------------------------------------

/// The session's channel. After `close` nothing more is sent (a closing session is silent).
pub struct Sink {
    channel: Channel<InvokeResponseBody>,
    closed: AtomicBool,
}

impl Sink {
    pub fn new(channel: Channel<InvokeResponseBody>) -> Self {
        Self {
            channel,
            closed: AtomicBool::new(false),
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }

    /// A send error means the webview is gone; there is nobody to tell.
    fn event(&self, event: &UiEvent) {
        if self.closed.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(text) = serde_json::to_string(event) {
            let _ = self.channel.send(InvokeResponseBody::Json(text));
        }
    }

    fn state(&self, state: ConnState, detail: Option<String>) {
        self.event(&UiEvent::State {
            state,
            detail,
            exit_status: None,
            ended_cleanly: false,
        });
    }

    /// The disconnect that ends an attempt or a session.
    fn ended(&self, detail: String, exit_status: Option<u32>, ended_cleanly: bool) {
        self.event(&UiEvent::State {
            state: ConnState::Disconnected,
            detail: Some(detail),
            exit_status,
            ended_cleanly,
        });
    }

    fn output(&self, bytes: Vec<u8>) {
        if !self.closed.load(Ordering::SeqCst) {
            let _ = self.channel.send(InvokeResponseBody::Raw(bytes));
        }
    }
}

/// Most keyboard-interactive prompts shown in one round.
const KI_MAX_PROMPTS: usize = 16;
/// Longest text of one prompt and of the round's name (characters).
const KI_MAX_PROMPT_CHARS: usize = 1024;
/// Longest instruction text of a round (characters).
const KI_MAX_INSTRUCTION_CHARS: usize = 4096;

/// `text` cut to `max` characters, an ellipsis at the end when something was cut.
fn cap_chars(text: String, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text,
        Some((end, _)) => format!("{}\u{2026}", &text[..end]),
    }
}

/// The prompt view of a server's keyboard-interactive round with its sizes bounded: at most 16
/// prompts of 1024 characters each and 4096 characters of instructions, so a hostile server cannot
/// flood the dialog. Only the view is capped; the caller still checks the answer against the
/// original prompt count, so a capped round cannot be answered in full and the login fails the
/// safe way instead of sending answers to prompts the user never saw.
fn cap_ki_round(round: KiRound) -> PromptView {
    PromptView::KeyboardInteractive {
        name: cap_chars(round.name, KI_MAX_PROMPT_CHARS),
        instructions: cap_chars(round.instructions, KI_MAX_INSTRUCTION_CHARS),
        prompts: round
            .prompts
            .into_iter()
            .take(KI_MAX_PROMPTS)
            .map(|p| KiPromptView {
                text: cap_chars(p.text, KI_MAX_PROMPT_CHARS),
                echo: p.echo,
            })
            .collect(),
    }
}

/// Answers the SSH layer's questions through prompt events. A dropped or closed hub, a cancel
/// or an answer of the wrong shape is the safe answer (reject, no secret).
pub struct TauriInteraction {
    pub hub: Arc<PromptHub>,
    pub sink: Arc<Sink>,
}

impl TauriInteraction {
    pub(crate) async fn ask(&self, prompt: PromptView) -> Option<PromptAnswer> {
        let (id, rx) = self.hub.register()?;
        self.sink.event(&UiEvent::Prompt { id, prompt });
        rx.await.ok()
    }

    pub(crate) async fn ask_secrets(&self, prompt: PromptView) -> Option<Vec<SecretString>> {
        match self.ask(prompt).await? {
            PromptAnswer::Secrets { values } => Some(values),
            _ => None,
        }
    }
}

impl Interaction for TauriInteraction {
    async fn host_key(&self, prompt: HostKeyPrompt) -> HostKeyDecision {
        let view = PromptView::HostKey {
            host: prompt.host,
            port: prompt.port,
            key_type: prompt.key_type,
            fingerprint: prompt.fingerprint,
        };
        match self.ask(view).await {
            Some(PromptAnswer::HostKey { decision }) => match decision {
                HostKeyAnswer::Once => HostKeyDecision::TrustOnce,
                HostKeyAnswer::Always => HostKeyDecision::TrustAlways,
                HostKeyAnswer::Reject => HostKeyDecision::Reject,
            },
            _ => HostKeyDecision::Reject,
        }
    }

    async fn host_key_synced(&self, prompt: HostKeySyncedPrompt) -> HostKeyDecision {
        let view = PromptView::HostKeySynced {
            host: prompt.host,
            port: prompt.port,
            key_type: prompt.key_type,
            fingerprint: prompt.fingerprint,
            previous: prompt.previous,
        };
        match self.ask(view).await {
            Some(PromptAnswer::HostKey { decision }) => match decision {
                HostKeyAnswer::Once => HostKeyDecision::TrustOnce,
                HostKeyAnswer::Always => HostKeyDecision::TrustAlways,
                HostKeyAnswer::Reject => HostKeyDecision::Reject,
            },
            _ => HostKeyDecision::Reject,
        }
    }

    async fn host_key_changed(&self, prompt: HostKeyChangedPrompt) -> HostKeyChangedDecision {
        let view = PromptView::HostKeyChanged {
            host: prompt.host,
            port: prompt.port,
            key_type: prompt.key_type,
            old: prompt.old,
            new: prompt.new,
        };
        match self.ask(view).await {
            Some(PromptAnswer::HostKeyChanged {
                decision: HostKeyChangedAnswer::Replace,
            }) => HostKeyChangedDecision::Replace,
            _ => HostKeyChangedDecision::Reject,
        }
    }

    async fn keyboard_interactive(&self, round: KiRound) -> Option<Vec<SecretString>> {
        // The server decides how many prompts it sends; the UI is shown a capped copy, and the
        // answer must still match the count the server asked for.
        let expected = round.prompts.len();
        let view = cap_ki_round(round);
        self.ask_secrets(view)
            .await
            .filter(|values| values.len() == expected)
    }

    async fn password(&self, user: &str, host: &str) -> Option<SecretString> {
        let view = PromptView::Password {
            user: user.to_string(),
            host: host.to_string(),
        };
        self.ask_secrets(view).await?.into_iter().next()
    }

    async fn username(&self, host: &str) -> Option<String> {
        let view = PromptView::Username {
            host: host.to_string(),
        };
        match self.ask(view).await? {
            PromptAnswer::Text { value } => {
                Some(value.trim().to_string()).filter(|name| !name.is_empty())
            }
            _ => None,
        }
    }

    async fn key_passphrase(&self) -> Option<SecretString> {
        self.ask_secrets(PromptView::KeyPassphrase {})
            .await?
            .into_iter()
            .next()
    }
}

// ---------------------------------------------------------------------------------------------
// Session manager
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Connecting,
    NeedsVault,
    Connected,
    Disconnected,
}

/// What the ordered writer task does with the live session. `Resize` carries no size: the writer
/// applies the latest `Shared::size` (character cells for a terminal, pixels for an RDP desktop),
/// see [`take_resize`]. `Rdp` and `Clipboard` only exist for RDP and `Data` only for SSH (the
/// other protocol's writer ignores them).
enum Op {
    Data(Vec<u8>),
    Rdp(Vec<fernjo_rdp::InputEvent>),
    Clipboard(String),
    Resize,
    Close,
}

impl Op {
    /// Bytes this op counts against the session's `InputBudget`: input ops weigh their length but
    /// at least [`MIN_OP_WEIGHT`], so the number of queued ops is bounded too. Resize and close
    /// weigh 0 and are never refused: at most one resize is queued at a time (`resize_queued`) and
    /// the close request is sent once, when the session is forgotten.
    fn weight(&self) -> usize {
        let bytes = match self {
            Op::Data(data) => data.len(),
            Op::Rdp(events) => events.len() * std::mem::size_of::<fernjo_rdp::InputEvent>(),
            Op::Clipboard(text) => text.len(),
            Op::Resize | Op::Close => return 0,
        };
        bytes.max(MIN_OP_WEIGHT)
    }
}

/// State shared by the manager, the connect task and the interaction.
struct Shared {
    sink: Arc<Sink>,
    hub: Arc<PromptHub>,
    flow: Mutex<FlowControl>,
    /// Wakes the forwarder when an acknowledgement may let it send again.
    wake: Notify,
    phase: Mutex<Phase>,
    size: Mutex<(u32, u32)>,
    /// Input, resizes and the close request, in order, towards the live `Session`. `None` while
    /// there is no shell. Lock order: `ops` before `size`.
    ops: Mutex<Option<UnboundedSender<Op>>>,
    /// Input bytes queued in `ops` and not yet handled by the writer.
    input_budget: InputBudget,
    /// An `Op::Resize` is in `ops` and the writer has not taken its size yet. Set and cleared
    /// only while `size` is locked, so a new size is either read by the queued resize or queues
    /// a new one (see [`take_resize`]).
    resize_queued: AtomicBool,
}

impl Shared {
    /// Queues `op` for the writer. Input is reserved against the budget first and the reservation
    /// goes back when the op was not queued; the writer releases it after handling the op.
    fn queue(&self, op: Op, no_session: impl Fn() -> CommandError) -> Result<(), CommandError> {
        let ops = lock(&self.ops)?;
        let tx = ops.as_ref().ok_or_else(&no_session)?;
        let weight = op.weight();
        if !self.input_budget.try_reserve(weight) {
            return Err(error(
                "input_queue_full",
                "too much input is waiting; try again",
            ));
        }
        tx.send(op).map_err(|_| {
            self.input_budget.release(weight);
            no_session()
        })
    }

    fn new(channel: Channel<InvokeResponseBody>, phase: Phase, size: (u32, u32)) -> Self {
        Self {
            sink: Arc::new(Sink::new(channel)),
            hub: Arc::new(PromptHub::default()),
            flow: Mutex::new(FlowControl::default()),
            wake: Notify::new(),
            phase: Mutex::new(phase),
            size: Mutex::new(size),
            ops: Mutex::new(None),
            input_budget: InputBudget::default(),
            resize_queued: AtomicBool::new(false),
        }
    }

    fn set_phase(&self, phase: Phase) {
        if let Ok(mut p) = self.phase.lock() {
            *p = phase;
        }
    }

    fn phase(&self) -> Result<Phase, CommandError> {
        Ok(*lock(&self.phase)?)
    }

    /// Ends the attempt: phase first, so a reconnect issued on seeing the event is accepted.
    fn disconnected(&self, ended: Ended) {
        self.set_phase(Phase::Disconnected);
        self.sink.ended(ended.text, ended.exit_status, ended.clean);
    }

    /// A failed attempt: no shell ran, so there is no status and nothing ended cleanly.
    fn failed(&self, text: String) {
        self.disconnected(Ended {
            text,
            exit_status: None,
            clean: false,
        });
    }
}

struct Entry {
    target: Target,
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// All sessions of the app, by id.
pub struct SessionManager {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
    /// Hosts confirmed during this run, used when the device has no folder for the persistent
    /// file (see `confirmed_layer`).
    confirmed_in_memory: KnownHosts,
    /// The RDP certificate pins of this run when the device has no folder for the file.
    rdp_certs_in_memory: fernjo_rdp::CertStore,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
            confirmed_in_memory: KnownHosts::in_memory(),
            rdp_certs_in_memory: fernjo_rdp::CertStore::in_memory(),
        }
    }
}

/// Where a connect looks for the hosts and certificates this device confirmed.
struct HostFiles {
    known_hosts: PathBuf,
    confirmed_hosts: Option<PathBuf>,
    confirmed_in_memory: Option<KnownHosts>,
    /// The device's RDP certificate pins (SSH sessions ignore it).
    rdp_certs: fernjo_rdp::CertStore,
}

/// Name of the device-local file of confirmed hosts inside `AppState::device_dir`.
const CONFIRMED_HOSTS_FILE: &str = "known_hosts.confirmed";
/// Name of the device-local file of pinned RDP certificates inside `AppState::device_dir`.
const RDP_CERTS_FILE: &str = "rdp_certs.confirmed";

const GONE: &str = "the connection no longer exists";

/// How a connected session ended.
pub(crate) struct Ended {
    pub text: String,
    pub exit_status: Option<u32>,
    pub clean: bool,
}

/// Classifies the end of a session that was connected for `connected_for`. `clean` means the
/// user left the shell deliberately: exit status 0 and a session of at least
/// [`MIN_AUTO_CLOSE_SESSION`]. Anything else (another status, no status, a lost connection, a
/// close request) is not clean.
pub(crate) fn classify_end(reason: CloseReason, connected_for: Duration) -> Ended {
    let exit_status = match reason {
        CloseReason::Exit(code) => code,
        CloseReason::ConnectionLost | CloseReason::ClosedByUser => None,
    };
    let text = match reason {
        CloseReason::Exit(Some(code)) => format!("The remote shell ended (exit status {code})."),
        CloseReason::Exit(None) => "The remote shell ended.".to_string(),
        CloseReason::ConnectionLost => "The connection was lost.".to_string(),
        CloseReason::ClosedByUser => "The session was closed.".to_string(),
    };
    Ended {
        text,
        exit_status,
        clean: exit_status == Some(0) && connected_for >= MIN_AUTO_CLOSE_SESSION,
    }
}

impl SessionManager {
    /// The known hosts file in the synced data folder and this device's own layer: the file in
    /// the device folder, or, without one, a store that lives as long as the app runs (a host
    /// then asks once per run; a connect never fails because of it).
    fn host_files(&self, state: &AppState) -> Result<HostFiles, CommandError> {
        let known_hosts = known_hosts_path(state)?;
        Ok(match &state.device_dir {
            Some(dir) => HostFiles {
                known_hosts,
                confirmed_hosts: Some(dir.join(CONFIRMED_HOSTS_FILE)),
                confirmed_in_memory: None,
                rdp_certs: fernjo_rdp::CertStore::open(dir.join(RDP_CERTS_FILE)),
            },
            None => HostFiles {
                known_hosts,
                confirmed_hosts: None,
                confirmed_in_memory: Some(self.confirmed_in_memory.clone()),
                rdp_certs: self.rdp_certs_in_memory.clone(),
            },
        })
    }

    fn entry(&self, id: &str) -> Result<Arc<Entry>, CommandError> {
        lock(&self.entries)?.get(id).cloned().ok_or_else(not_found)
    }

    #[cfg(test)]
    pub(crate) fn weak_shared(&self, id: &str) -> Option<std::sync::Weak<impl Sized>> {
        self.entry(id).ok().map(|e| Arc::downgrade(&e.shared))
    }

    /// Resolves the connection and starts its session.
    ///
    /// # Errors
    /// What `connect_plan::resolve_session` reports (`not_found`, `not_a_connection`, ...), or
    /// `no_data_dir`. `cols` and `rows` are character cells for an SSH connection and pixels for
    /// an RDP one.
    pub fn open(
        &self,
        state: &AppState,
        node_id: NodeId,
        channel: Channel<InvokeResponseBody>,
        cols: u32,
        rows: u32,
    ) -> Result<String, CommandError> {
        self.open_target(state, Target::Node(node_id), channel, cols, rows)
    }

    /// Resolves the target (a tree node or a quick connect target) and starts its session.
    ///
    /// # Errors
    /// What `connect_plan::resolve_target` reports, or `no_data_dir`. A quick target is checked
    /// again here (`invalid_host`), whatever the webview parsed.
    pub fn open_target(
        &self,
        state: &AppState,
        target: Target,
        channel: Channel<InvokeResponseBody>,
        cols: u32,
        rows: u32,
    ) -> Result<String, CommandError> {
        let plan = connect_plan::resolve_target(state, &target, false)?;
        let hosts = self.host_files(state)?;
        let shared = Arc::new(Shared::new(channel, Phase::Connecting, (cols, rows)));
        let entry = Arc::new(Entry {
            target,
            shared,
            task: Mutex::new(None),
        });
        let id = uuid::Uuid::new_v4().to_string();
        lock(&self.entries)?.insert(id.clone(), Arc::clone(&entry));
        start(&entry, plan, hosts);
        Ok(id)
    }

    /// Creates a session entry in the disconnected state without connecting (a tab restored from
    /// the last run): it sends the disconnected event with [`RESTORED_TEXT`] (not clean, no exit
    /// status) and waits for `reconnect`, which resolves the plan then. It never counts as
    /// connected.
    ///
    /// # Errors
    /// `not_found` / `not_a_connection` for a node that is gone or a folder, a structure code for
    /// a bad quick target.
    pub fn restore(
        &self,
        state: &AppState,
        target: Target,
        channel: Channel<InvokeResponseBody>,
        cols: u32,
        rows: u32,
    ) -> Result<String, CommandError> {
        connect_plan::check_target(state, &target)?;
        let shared = Arc::new(Shared::new(channel, Phase::Disconnected, (cols, rows)));
        let entry = Arc::new(Entry {
            target,
            shared,
            task: Mutex::new(None),
        });
        let id = uuid::Uuid::new_v4().to_string();
        lock(&self.entries)?.insert(id.clone(), Arc::clone(&entry));
        entry.shared.failed(RESTORED_TEXT.to_string());
        Ok(id)
    }

    /// Connects again after the session ended. The plan is resolved from the tree as it is now.
    ///
    /// # Errors
    /// `not_found`; `session_active` unless the session is disconnected.
    pub fn reconnect(
        &self,
        state: &AppState,
        id: &str,
        cols: u32,
        rows: u32,
    ) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        if entry.shared.phase()? != Phase::Disconnected {
            return Err(error("session_active", "the session is still active"));
        }
        *lock(&entry.shared.size)? = (cols, rows);
        self.begin(state, &entry, false)
    }

    /// Continues a session that waits for the vault, after an unlock or with the credential
    /// skipped.
    ///
    /// # Errors
    /// `not_found`; `session_active` unless the session waits for the vault.
    pub fn resume(
        &self,
        state: &AppState,
        id: &str,
        skip_credential: bool,
    ) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        if entry.shared.phase()? != Phase::NeedsVault {
            return Err(error(
                "session_active",
                "the session is not waiting for the vault",
            ));
        }
        self.begin(state, &entry, skip_credential)
    }

    fn begin(
        &self,
        state: &AppState,
        entry: &Arc<Entry>,
        skip_vault: bool,
    ) -> Result<(), CommandError> {
        let plan = connect_plan::resolve_target(state, &entry.target, skip_vault);
        match (plan, self.host_files(state)) {
            (Ok(plan), Ok(hosts)) => start(entry, plan, hosts),
            (Err(e), _) | (_, Err(e)) => {
                // only a tree node can be gone
                let text = if e.code == "not_found" && matches!(entry.target, Target::Node(_)) {
                    GONE.to_string()
                } else {
                    e.message
                };
                entry.shared.failed(text);
            }
        }
        Ok(())
    }

    /// Queues keystrokes for the shell.
    ///
    /// # Errors
    /// `not_found`; `session_not_connected` while there is no shell.
    pub fn input(&self, id: &str, data: Vec<u8>) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        entry.shared.queue(Op::Data(data), || {
            error("session_not_connected", "the session has no running shell")
        })
    }

    /// Remembers the terminal size and tells the shell when there is one.
    ///
    /// # Errors
    /// `not_found`.
    pub fn resize(&self, id: &str, cols: u32, rows: u32) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        let shared = &entry.shared;
        let ops = lock(&shared.ops)?;
        let mut size = lock(&shared.size)?;
        *size = (cols, rows);
        // Without a shell the size is applied at the next connect. With one, a resize that is
        // still queued will read this size; otherwise one is queued now.
        if let Some(tx) = ops.as_ref()
            && !shared.resize_queued.swap(true, Ordering::SeqCst)
            && tx.send(Op::Resize).is_err()
        {
            shared.resize_queued.store(false, Ordering::SeqCst);
        }
        Ok(())
    }

    /// The UI wrote `bytes` of output to the terminal.
    ///
    /// # Errors
    /// `not_found`.
    pub fn ack(&self, id: &str, bytes: usize) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        lock(&entry.shared.flow)?.on_ack(bytes);
        entry.shared.wake.notify_one();
        Ok(())
    }

    /// Delivers a prompt answer.
    ///
    /// # Errors
    /// `not_found` for an unknown session or prompt.
    pub fn answer(
        &self,
        id: &str,
        prompt_id: u64,
        answer: PromptAnswer,
    ) -> Result<(), CommandError> {
        self.entry(id)?.shared.hub.answer(prompt_id, answer)
    }

    /// Ends the session and forgets it. Silent: no further event reaches the channel, and every
    /// open prompt is answered with the safe answer so a pending connect ends without trusting
    /// anything.
    ///
    /// # Errors
    /// `not_found`.
    pub fn close(&self, id: &str) -> Result<(), CommandError> {
        let entry = lock(&self.entries)?.remove(id).ok_or_else(not_found)?;
        shut_down(&entry);
        Ok(())
    }

    /// Ends every session, silently, like `close` for each. Called when the webview starts
    /// loading a page: a reload or a crash leaves the old page's sessions without a UI, and the
    /// page's own `closeAll` on `pagehide` is fire-and-forget and may never arrive.
    pub fn close_all(&self) {
        let entries: Vec<Arc<Entry>> = match self.entries.lock() {
            Ok(mut map) => map.drain().map(|(_, entry)| entry).collect(),
            Err(poisoned) => poisoned
                .into_inner()
                .drain()
                .map(|(_, entry)| entry)
                .collect(),
        };
        for entry in &entries {
            shut_down(entry);
        }
    }

    /// Number of sessions whose shell is connected. Sessions still connecting, waiting for the
    /// vault or already disconnected do not count: closing the window loses nothing there.
    pub fn connected_count(&self) -> usize {
        let entries: Vec<Arc<Entry>> = match self.entries.lock() {
            Ok(map) => map.values().cloned().collect(),
            Err(poisoned) => poisoned.into_inner().values().cloned().collect(),
        };
        entries
            .iter()
            .filter(|entry| matches!(entry.shared.phase(), Ok(Phase::Connected)))
            .count()
    }

    /// Number of sessions that exist (open, connecting, prompting or disconnected).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |map| map.len())
    }
}

/// Closing the main window asks first only while a session is connected.
pub(crate) fn needs_close_confirmation(connected: usize) -> bool {
    connected > 0
}

/// What an app exit request (`RunEvent::ExitRequested`, e.g. Cmd+Q on macOS) should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExitAction {
    /// Let the exit proceed.
    Allow,
    /// Stop the exit and show the confirmation dialog.
    Ask,
    /// Stop the exit; a dialog is already open.
    Prevent,
}

/// An explicit exit code (restart, update, `app.exit(n)`) and an exit without a connected
/// session are never intercepted; otherwise the exit waits for the dialog.
pub(crate) fn exit_action(code: Option<i32>, connected: usize, prompt_open: bool) -> ExitAction {
    if code.is_some() || !needs_close_confirmation(connected) {
        ExitAction::Allow
    } else if prompt_open {
        ExitAction::Prevent
    } else {
        ExitAction::Ask
    }
}

/// Mutes the channel, answers every prompt with the safe answer, tells the writer task to close
/// and aborts the connect task. For SSH the abort is what ends a session whose server stopped
/// reading: the writer may hang in a write and never take `Op::Close`, but aborting `run` drops
/// the events receiver, which makes `fernjo_ssh::Session` shut its socket down and fail that write
/// (M11 review #22).
fn shut_down(entry: &Entry) {
    entry.shared.sink.close();
    entry.shared.hub.close();
    if let Ok(ops) = entry.shared.ops.lock()
        && let Some(tx) = ops.as_ref()
    {
        let _ = tx.send(Op::Close);
    }
    if let Ok(mut task) = entry.task.lock()
        && let Some(handle) = task.take()
    {
        handle.abort();
    }
}

fn known_hosts_path(state: &AppState) -> Result<PathBuf, CommandError> {
    state
        .data_dir
        .as_ref()
        .map(|dir| dir.join("known_hosts"))
        .ok_or_else(|| error("no_data_dir", "there is no data folder for the known hosts"))
}

/// Starts the session for a resolved plan of either protocol.
fn start(entry: &Arc<Entry>, plan: Plan, hosts: HostFiles) {
    match plan {
        Plan::Ssh(plan) => start_ssh(entry, plan, hosts),
        Plan::Rdp(plan) => rdp::start(entry, plan, hosts.rdp_certs),
    }
}

/// Starts (or restarts) the connect task for a resolved plan, or parks the session until the
/// vault is unlocked.
fn start_ssh(entry: &Arc<Entry>, plan: ConnectPlan, hosts: HostFiles) {
    let shared = &entry.shared;
    let note = match &plan.credential {
        CredentialPlan::NeedsVault => {
            shared.set_phase(Phase::NeedsVault);
            shared.sink.state(ConnState::NeedsVault, None);
            return;
        }
        CredentialPlan::Missing(reason) => Some(reason.text().to_string()),
        CredentialPlan::None | CredentialPlan::Ready(_) => None,
    };
    shared.set_phase(Phase::Connecting);
    shared.sink.state(ConnState::Connecting, note);
    let (cols, rows) = shared.size.lock().map_or((80, 24), |s| *s);
    let mut params = ConnectParams::new(plan.host, plan.port, hosts.known_hosts);
    params.confirmed_hosts = hosts.confirmed_hosts;
    params.confirmed_in_memory = hosts.confirmed_in_memory;
    params.username = plan.username;
    params.keepalive_secs = plan.keepalive_secs;
    params.cols = cols;
    params.rows = rows;
    if let CredentialPlan::Ready(credential) = plan.credential {
        params.credential = Some(credential);
    }
    let handle = tauri::async_runtime::spawn(run(Arc::clone(shared), params));
    if let Ok(mut task) = entry.task.lock() {
        *task = Some(handle);
    }
}

/// One connection attempt and, once connected, the output forwarder.
async fn run(shared: Arc<Shared>, params: ConnectParams) {
    let interaction = Arc::new(TauriInteraction {
        hub: Arc::clone(&shared.hub),
        sink: Arc::clone(&shared.sink),
    });
    let started_size = (params.cols, params.rows);
    let (session, rx) = match Session::connect(params, interaction).await {
        Ok(connected) => connected,
        Err(e) => {
            shared.failed(e.to_string());
            return;
        }
    };
    let (tx, ops) = unbounded_channel();
    install_ops(&shared, tx, |size| size != started_size);
    tauri::async_runtime::spawn(write_ops(session, ops, Arc::clone(&shared)));
    if let Ok(mut flow) = shared.flow.lock() {
        *flow = FlowControl::default();
    }
    shared.set_phase(Phase::Connected);
    shared.sink.state(ConnState::Connected, None);
    let connected_at = Instant::now();
    let ended = forward(&shared, rx, connected_at).await;
    if let Ok(mut slot) = shared.ops.lock() {
        // Dropping the sender ends the writer, which drops (and so closes) the session.
        *slot = None;
    }
    shared.disconnected(ended);
}

/// Applies input, resizes and the close request to the session, in the order they were queued.
async fn write_ops(session: Session, mut ops: UnboundedReceiver<Op>, shared: Arc<Shared>) {
    while let Some(op) = ops.recv().await {
        let weight = op.weight();
        let stop = match op {
            Op::Data(data) => session.write(data).await.is_err(),
            Op::Resize => {
                let (cols, rows) = take_resize(&shared);
                let _ = session.resize(cols, rows).await;
                false
            }
            // RDP only
            Op::Rdp(_) | Op::Clipboard(_) => false,
            Op::Close => {
                session.close();
                true
            }
        };
        shared.input_budget.release(weight);
        if stop {
            break;
        }
    }
    release_unhandled(&mut ops, &shared);
}

/// Makes `tx` the session's op queue once the shell (or desktop) is live, and queues a resize
/// when `differs` says the size the UI reported meanwhile is not the one the connect used. Runs
/// under the `ops` and `size` locks, so a resize that arrives during this call is either seen
/// here or queued by `SessionManager::resize` on the new sender. It also resets a flag left set
/// by a writer that stopped with a resize still queued.
fn install_ops(shared: &Shared, tx: UnboundedSender<Op>, differs: impl Fn((u32, u32)) -> bool) {
    let Ok(mut slot) = shared.ops.lock() else {
        return;
    };
    let resize = shared.size.lock().is_ok_and(|size| {
        let wanted = differs(*size);
        shared.resize_queued.store(wanted, Ordering::SeqCst);
        wanted
    });
    if resize {
        let _ = tx.send(Op::Resize);
    }
    *slot = Some(tx);
}

/// The writer's half of resize coalescing: clears `resize_queued` and returns the latest size, both
/// under the `size` lock. `SessionManager::resize` stores the size and tests the flag under the
/// same lock, so a size stored before this call is returned here, and one stored after it finds
/// the flag clear and queues a new `Op::Resize`: the final size always reaches the session, and
/// the queue never holds more than one resize.
fn take_resize(shared: &Shared) -> (u32, u32) {
    match shared.size.lock() {
        Ok(size) => {
            shared.resize_queued.store(false, Ordering::SeqCst);
            *size
        }
        Err(poisoned) => {
            shared.resize_queued.store(false, Ordering::SeqCst);
            *poisoned.into_inner()
        }
    }
}

/// Gives back the budget of ops that were queued but never handled (the writer stopped), so a
/// reconnect of the same tab starts with an empty budget.
fn release_unhandled(ops: &mut UnboundedReceiver<Op>, shared: &Shared) {
    ops.close();
    while let Ok(op) = ops.try_recv() {
        shared.input_budget.release(op.weight());
    }
}

/// Waits until the flow control lets output through.
async fn wait_for_credit(shared: &Shared) {
    loop {
        let open = shared.flow.lock().map_or(true, |f| f.should_send());
        if open {
            return;
        }
        // `notify_one` keeps a permit when nobody waits, so an ack between the check and this
        // await is not lost.
        shared.wake.notified().await;
    }
}

/// Sends the collected bytes to the UI in messages of at most [`MAX_BATCH`].
fn flush(shared: &Shared, pending: &mut Vec<u8>) {
    if pending.is_empty() {
        return;
    }
    let bytes = std::mem::take(pending);
    for chunk in bytes.chunks(MAX_BATCH) {
        if let Ok(mut flow) = shared.flow.lock() {
            flow.on_sent(chunk.len());
        }
        shared.sink.output(chunk.to_vec());
    }
}

/// Forwards output in batches while the flow control allows, until the session ends. While the
/// UI is behind, nothing is read from the SSH layer, so its bounded queue fills and the server
/// is held back by the SSH window: memory stays bounded and Ctrl+C is not stuck behind a backlog.
async fn forward(shared: &Shared, mut rx: Receiver<SessionEvent>, connected_at: Instant) -> Ended {
    let mut pending: Vec<u8> = Vec::new();
    let mut deadline = Instant::now();
    loop {
        if pending.is_empty() {
            wait_for_credit(shared).await;
        }
        let event = if pending.is_empty() {
            rx.recv().await
        } else {
            tokio::select! {
                event = rx.recv() => event,
                () = tokio::time::sleep_until(deadline) => {
                    flush(shared, &mut pending);
                    continue;
                }
            }
        };
        match event {
            Some(SessionEvent::Output(bytes)) => {
                if pending.is_empty() {
                    deadline = Instant::now() + BATCH_INTERVAL;
                }
                pending.extend(bytes);
                if pending.len() >= MAX_BATCH {
                    flush(shared, &mut pending);
                }
            }
            Some(SessionEvent::Closed(reason)) => {
                flush(shared, &mut pending);
                return classify_end(reason, connected_at.elapsed());
            }
            None => {
                flush(shared, &mut pending);
                return classify_end(CloseReason::ConnectionLost, connected_at.elapsed());
            }
        }
    }
}

mod rdp;

#[cfg(test)]
mod tests;
