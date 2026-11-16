//! The session manager: terminal sessions, their prompts and their output flow.
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
//!   ([`UiEvent`]): `{kind:"state", state, detail}` or `{kind:"prompt", id, prompt}`.
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
    HostKeyPrompt, Interaction, KiRound, Session, SessionEvent,
};
use secrecy::SecretString;
use serde::{Deserialize, Deserializer, Serialize};
use tauri::async_runtime::JoinHandle;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc::{Receiver, UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Notify, oneshot};
use tokio::time::Instant;

use crate::commands::CommandError;
use crate::connect_plan::{self, ConnectPlan, CredentialPlan};
use crate::session_flow::FlowControl;
use crate::state::AppState;

/// Largest raw output message.
pub const MAX_BATCH: usize = 32 * 1024;
/// How long output is collected before it is sent.
pub const BATCH_INTERVAL: Duration = Duration::from_millis(16);

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
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UiEvent {
    State {
        state: ConnState,
        detail: Option<String>,
    },
    Prompt {
        id: u64,
        prompt: PromptView,
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
        self.event(&UiEvent::State { state, detail });
    }

    fn output(&self, bytes: Vec<u8>) {
        if !self.closed.load(Ordering::SeqCst) {
            let _ = self.channel.send(InvokeResponseBody::Raw(bytes));
        }
    }
}

/// Answers the SSH layer's questions through prompt events. A dropped or closed hub, a cancel
/// or an answer of the wrong shape is the safe answer (reject, no secret).
pub struct TauriInteraction {
    pub hub: Arc<PromptHub>,
    pub sink: Arc<Sink>,
}

impl TauriInteraction {
    async fn ask(&self, prompt: PromptView) -> Option<PromptAnswer> {
        let (id, rx) = self.hub.register()?;
        self.sink.event(&UiEvent::Prompt { id, prompt });
        rx.await.ok()
    }

    async fn ask_secrets(&self, prompt: PromptView) -> Option<Vec<SecretString>> {
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
        let expected = round.prompts.len();
        let view = PromptView::KeyboardInteractive {
            name: round.name,
            instructions: round.instructions,
            prompts: round
                .prompts
                .into_iter()
                .map(|p| KiPromptView {
                    text: p.text,
                    echo: p.echo,
                })
                .collect(),
        };
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

/// What the ordered writer task does with the live `Session`.
enum Op {
    Data(Vec<u8>),
    Resize(u32, u32),
    Close,
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
    /// there is no shell.
    ops: Mutex<Option<UnboundedSender<Op>>>,
}

impl Shared {
    fn set_phase(&self, phase: Phase) {
        if let Ok(mut p) = self.phase.lock() {
            *p = phase;
        }
    }

    fn phase(&self) -> Result<Phase, CommandError> {
        Ok(*lock(&self.phase)?)
    }

    /// Ends the attempt: phase first, so a reconnect issued on seeing the event is accepted.
    fn disconnected(&self, detail: String) {
        self.set_phase(Phase::Disconnected);
        self.sink.state(ConnState::Disconnected, Some(detail));
    }
}

struct Entry {
    node_id: NodeId,
    shared: Arc<Shared>,
    task: Mutex<Option<JoinHandle<()>>>,
}

/// All sessions of the app, by id.
#[derive(Default)]
pub struct SessionManager {
    entries: Mutex<HashMap<String, Arc<Entry>>>,
}

const GONE: &str = "the connection no longer exists";

fn close_text(reason: CloseReason) -> String {
    match reason {
        CloseReason::Exit(Some(code)) => format!("The remote shell ended (exit status {code})."),
        CloseReason::Exit(None) => "The remote shell ended.".to_string(),
        CloseReason::ConnectionLost => "The connection was lost.".to_string(),
        CloseReason::ClosedByUser => "The session was closed.".to_string(),
    }
}

impl SessionManager {
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
    /// What `connect_plan::resolve` reports (`not_found`, `not_a_connection`, ...), or
    /// `no_data_dir`.
    pub fn open(
        &self,
        state: &AppState,
        node_id: NodeId,
        channel: Channel<InvokeResponseBody>,
        cols: u32,
        rows: u32,
    ) -> Result<String, CommandError> {
        let plan = connect_plan::resolve(state, node_id, false)?;
        let known_hosts = known_hosts_path(state)?;
        let shared = Arc::new(Shared {
            sink: Arc::new(Sink::new(channel)),
            hub: Arc::new(PromptHub::default()),
            flow: Mutex::new(FlowControl::default()),
            wake: Notify::new(),
            phase: Mutex::new(Phase::Connecting),
            size: Mutex::new((cols, rows)),
            ops: Mutex::new(None),
        });
        let entry = Arc::new(Entry {
            node_id,
            shared,
            task: Mutex::new(None),
        });
        let id = uuid::Uuid::new_v4().to_string();
        lock(&self.entries)?.insert(id.clone(), Arc::clone(&entry));
        start(&entry, plan, known_hosts);
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
        let plan = connect_plan::resolve(state, entry.node_id, skip_vault);
        match (plan, known_hosts_path(state)) {
            (Ok(plan), Ok(known_hosts)) => start(entry, plan, known_hosts),
            (Err(e), _) | (_, Err(e)) => {
                let text = if e.code == "not_found" {
                    GONE.to_string()
                } else {
                    e.message
                };
                entry.shared.disconnected(text);
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
        let ops = lock(&entry.shared.ops)?;
        let no_shell = || error("session_not_connected", "the session has no running shell");
        ops.as_ref()
            .ok_or_else(no_shell)?
            .send(Op::Data(data))
            .map_err(|_| no_shell())
    }

    /// Remembers the terminal size and tells the shell when there is one.
    ///
    /// # Errors
    /// `not_found`.
    pub fn resize(&self, id: &str, cols: u32, rows: u32) -> Result<(), CommandError> {
        let entry = self.entry(id)?;
        *lock(&entry.shared.size)? = (cols, rows);
        // Without a shell the size is applied at the next connect.
        if let Some(tx) = lock(&entry.shared.ops)?.as_ref() {
            let _ = tx.send(Op::Resize(cols, rows));
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

    /// Number of sessions that exist (open, connecting, prompting or disconnected).
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.lock().map_or(0, |map| map.len())
    }
}

/// Mutes the channel, answers every prompt with the safe answer, tells the writer task to close
/// and aborts the connect task.
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

/// Starts (or restarts) the connect task for a resolved plan, or parks the session until the
/// vault is unlocked.
fn start(entry: &Arc<Entry>, plan: ConnectPlan, known_hosts: PathBuf) {
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
    let mut params = ConnectParams::new(plan.host, plan.port, known_hosts);
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
            shared.disconnected(e.to_string());
            return;
        }
    };
    let (tx, ops) = unbounded_channel();
    // A resize that arrived while connecting is applied now.
    if let Ok(size) = shared.size.lock()
        && *size != started_size
    {
        let _ = tx.send(Op::Resize(size.0, size.1));
    }
    if let Ok(mut slot) = shared.ops.lock() {
        *slot = Some(tx);
    }
    tauri::async_runtime::spawn(write_ops(session, ops));
    if let Ok(mut flow) = shared.flow.lock() {
        *flow = FlowControl::default();
    }
    shared.set_phase(Phase::Connected);
    shared.sink.state(ConnState::Connected, None);
    let detail = forward(&shared, rx).await;
    if let Ok(mut slot) = shared.ops.lock() {
        // Dropping the sender ends the writer, which drops (and so closes) the session.
        *slot = None;
    }
    shared.disconnected(detail);
}

/// Applies input, resizes and the close request to the session, in the order they were queued.
async fn write_ops(session: Session, mut ops: UnboundedReceiver<Op>) {
    while let Some(op) = ops.recv().await {
        match op {
            Op::Data(data) => {
                if session.write(data).await.is_err() {
                    break;
                }
            }
            Op::Resize(cols, rows) => {
                let _ = session.resize(cols, rows).await;
            }
            Op::Close => {
                session.close();
                break;
            }
        }
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
async fn forward(shared: &Shared, mut rx: Receiver<SessionEvent>) -> String {
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
                return close_text(reason);
            }
            None => {
                flush(shared, &mut pending);
                return close_text(CloseReason::ConnectionLost);
            }
        }
    }
}

#[cfg(test)]
mod tests;
