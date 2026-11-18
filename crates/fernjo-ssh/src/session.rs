//! One SSH connection with an interactive shell on a PTY.

use std::borrow::Cow;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::client::{self, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::agent::AgentIdentity;
use russh::keys::ssh_key::{HashAlg, PublicKey};
use russh::keys::{Algorithm, PrivateKey, PrivateKeyWithHashAlg};
use russh::{ChannelMsg, ChannelWriteHalf, Disconnect};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};

use crate::SshError;
use crate::agent::{self, Agent};
use crate::auth_plan::{Attempt, AuthPlanner, Available, Offered};
use crate::handler::{ClientHandler, Rejection};
use crate::interaction::{Interaction, KiPrompt, KiRound};
use crate::known_hosts::KnownHosts;
use crate::params::{ConnectParams, Credential};

/// Entries the events channel holds before the session task blocks (backpressure towards the server).
const EVENT_QUEUE: usize = 4;
/// Keyboard-interactive rounds one attempt may take before it counts as failed.
const MAX_KI_ROUNDS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReason {
    /// The remote shell ended, with its exit status when the server sent one.
    Exit(Option<u32>),
    ConnectionLost,
    ClosedByUser,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionEvent {
    Output(Vec<u8>),
    Closed(CloseReason),
}

/// Handle on a running shell session. Output arrives on the receiver `connect` returned.
pub struct Session {
    writer: ChannelWriteHalf<Msg>,
    stop: Arc<Notify>,
    closed: Arc<AtomicBool>,
}

pub(crate) fn client_config(keepalive_secs: u32) -> client::Config {
    // russh's default host key list ends with plain `ssh-rsa` (RSA over SHA-1), which OpenSSH
    // stopped offering in 8.8. It would only be used for a host seen for the first time, but a
    // SHA-1 signature is not worth the compatibility with servers that sign nothing better.
    let mut preferred = client::Config::default().preferred;
    let host_keys: Vec<Algorithm> = preferred
        .key
        .iter()
        .filter(|alg| **alg != Algorithm::Rsa { hash: None })
        .cloned()
        .collect();
    preferred.key = Cow::Owned(host_keys);
    client::Config {
        preferred,
        keepalive_interval: (keepalive_secs > 0)
            .then(|| Duration::from_secs(u64::from(keepalive_secs))),
        keepalive_max: 3,
        nodelay: true,
        // Everything between the server and the terminal must drain before Ctrl+C is echoed, so the
        // data in flight is kept small: SSH window + packets buffered in the channel + EVENT_QUEUE
        // events (see Docs/DECISIONS.md, "M4 flow control numbers").
        window_size: 512 * 1024,
        channel_buffer_size: 4,
        ..client::Config::default()
    }
}

/// Offer only the host key algorithms already stored for this host, so a server cannot dodge the
/// stored key by presenting a different type (OpenSSH does the same).
fn restrict_host_key_algorithms(config: &mut client::Config, stored: &[Algorithm]) {
    if stored.is_empty() {
        return;
    }
    let mut list: Vec<Algorithm> = Vec::new();
    for alg in stored {
        match alg {
            // A stored RSA key is signed with SHA-2 today; plain `ssh-rsa` (SHA-1) is not offered.
            Algorithm::Rsa { .. } => {
                for hash in [HashAlg::Sha512, HashAlg::Sha256] {
                    list.push(Algorithm::Rsa { hash: Some(hash) });
                }
            }
            other => list.push(other.clone()),
        }
    }
    config.preferred.key = Cow::Owned(list);
}

fn offered_from(methods: &russh::MethodSet) -> Offered {
    use russh::MethodKind;
    Offered {
        publickey: methods.contains(&MethodKind::PublicKey),
        password: methods.contains(&MethodKind::Password),
        keyboard_interactive: methods.contains(&MethodKind::KeyboardInteractive),
    }
}

enum Outcome {
    Success,
    /// Rejected; the methods that may still continue.
    Rejected(Offered),
    /// Nothing reached the server (identity not signable, key unreadable): the server's methods
    /// are as before.
    Skipped,
}

fn auth_failed() -> SshError {
    SshError::AuthFailed {
        key_unreadable: false,
    }
}

/// An `AuthFailed` raised inside an attempt does not know about an unreadable key; add that.
fn with_key_flag(error: SshError, key_unreadable: bool) -> SshError {
    match error {
        SshError::AuthFailed { .. } => SshError::AuthFailed { key_unreadable },
        other => other,
    }
}

fn io_to_lost(_: russh::Error) -> SshError {
    SshError::ConnectionLost
}

/// Bounds one wait for the server (never a user prompt) by the connect timeout.
async fn bounded<T>(
    limit: Duration,
    fut: impl Future<Output = Result<T, russh::Error>>,
) -> Result<T, SshError> {
    match tokio::time::timeout(limit, fut).await {
        Ok(result) => result.map_err(io_to_lost),
        Err(_) => Err(SshError::Timeout),
    }
}

async fn password_attempt<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    password: &SecretString,
    limit: Duration,
) -> Result<Outcome, SshError> {
    match bounded(
        limit,
        handle.authenticate_password(user, password.expose_secret().to_owned()),
    )
    .await?
    {
        russh::client::AuthResult::Success => Ok(Outcome::Success),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => Ok(Outcome::Rejected(offered_from(&remaining_methods))),
    }
}

/// The server side of a keyboard-interactive exchange, so the round loop can be tested without a
/// server.
trait KiTransport: Send {
    fn respond(
        &mut self,
        answers: Vec<String>,
    ) -> impl Future<Output = Result<KeyboardInteractiveAuthResponse, SshError>> + Send;
}

struct HandleKi<'a, H: client::Handler> {
    handle: &'a mut client::Handle<H>,
    limit: Duration,
}

impl<H: client::Handler> KiTransport for HandleKi<'_, H> {
    async fn respond(
        &mut self,
        answers: Vec<String>,
    ) -> Result<KeyboardInteractiveAuthResponse, SshError> {
        bounded(
            self.limit,
            self.handle
                .authenticate_keyboard_interactive_respond(answers),
        )
        .await
    }
}

async fn keyboard_interactive_attempt<H: client::Handler, I: Interaction>(
    handle: &mut client::Handle<H>,
    user: &str,
    interaction: &I,
    limit: Duration,
) -> Result<Outcome, SshError> {
    let first = bounded(
        limit,
        handle.authenticate_keyboard_interactive_start(user, None),
    )
    .await?;
    keyboard_interactive_rounds(first, &mut HandleKi { handle, limit }, interaction).await
}

/// Works through the server's `InfoRequest` rounds. The user prompts (`interaction`) are never
/// inside a bounded wait; only `transport.respond` is.
async fn keyboard_interactive_rounds<T: KiTransport, I: Interaction>(
    first: KeyboardInteractiveAuthResponse,
    transport: &mut T,
    interaction: &I,
) -> Result<Outcome, SshError> {
    let mut response = first;
    // Answers sent so far. The reply to the last allowed answer is still judged below; only a
    // further request for input is refused.
    let mut answered = 0;
    loop {
        match response {
            KeyboardInteractiveAuthResponse::Success => return Ok(Outcome::Success),
            KeyboardInteractiveAuthResponse::Failure {
                remaining_methods, ..
            } => return Ok(Outcome::Rejected(offered_from(&remaining_methods))),
            KeyboardInteractiveAuthResponse::InfoRequest {
                name,
                instructions,
                prompts,
            } => {
                if answered == MAX_KI_ROUNDS {
                    return Ok(Outcome::Rejected(Offered {
                        publickey: false,
                        password: false,
                        keyboard_interactive: false,
                    }));
                }
                answered += 1;
                let answers: Vec<String> = if prompts.is_empty() {
                    Vec::new()
                } else {
                    let round = KiRound {
                        name,
                        instructions,
                        prompts: prompts
                            .into_iter()
                            .map(|p| KiPrompt {
                                text: p.prompt,
                                echo: p.echo,
                            })
                            .collect(),
                    };
                    let expected = round.prompts.len();
                    // `None` means the user cancelled.
                    let answers = interaction
                        .keyboard_interactive(round)
                        .await
                        .ok_or_else(auth_failed)?;
                    if answers.len() != expected {
                        return Err(SshError::Protocol);
                    }
                    answers
                        .iter()
                        .map(|a| a.expose_secret().to_owned())
                        .collect()
                };
                response = transport.respond(answers).await?;
            }
        }
    }
}

/// Why a stored key could not be turned into a usable key. Carries no key text.
enum KeyProblem {
    Encrypted,
    Unreadable,
}

fn decode_key(
    text: &SecretString,
    passphrase: Option<&SecretString>,
) -> Result<PrivateKey, KeyProblem> {
    match russh::keys::decode_secret_key(
        text.expose_secret(),
        passphrase.map(ExposeSecret::expose_secret),
    ) {
        Ok(key) => Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => Err(KeyProblem::Encrypted),
        Err(_) => Err(KeyProblem::Unreadable),
    }
}

/// Decodes the credential key; an encrypted key without a stored passphrase asks the user once.
/// A wrong stored passphrase, a cancelled prompt and garbage all count as unreadable (`None`).
async fn load_key<I: Interaction>(
    text: &SecretString,
    stored_passphrase: Option<&SecretString>,
    interaction: &I,
) -> Option<PrivateKey> {
    match decode_key(text, stored_passphrase) {
        Ok(key) => Some(key),
        Err(KeyProblem::Unreadable) => None,
        Err(KeyProblem::Encrypted) if stored_passphrase.is_some() => None,
        Err(KeyProblem::Encrypted) => {
            let typed = interaction.key_passphrase().await?;
            decode_key(text, Some(&typed)).ok()
        }
    }
}

/// Reads the agent's plain public keys (certificates are not offered). Bounded; any failure means
/// no identities.
async fn agent_identities(agent: &mut Agent, limit: Duration) -> Vec<PublicKey> {
    match tokio::time::timeout(limit, agent.request_identities()).await {
        Ok(Ok(all)) => all
            .into_iter()
            .filter_map(|id| match id {
                AgentIdentity::PublicKey { key, .. } => Some(key),
                AgentIdentity::Certificate { .. } => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Hash for RSA signatures: what the server supports best (SHA-512, then SHA-256), SHA-256 when it
/// did not announce its algorithms. Never SHA-1 unless the server offers nothing else.
async fn rsa_hash<H: client::Handler>(
    handle: &client::Handle<H>,
    limit: Duration,
) -> Result<Option<HashAlg>, SshError> {
    Ok(bounded(limit, handle.best_supported_rsa_hash())
        .await?
        .unwrap_or(Some(HashAlg::Sha256)))
}

fn outcome_of(result: russh::client::AuthResult) -> Outcome {
    match result {
        russh::client::AuthResult::Success => Outcome::Success,
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => Outcome::Rejected(offered_from(&remaining_methods)),
    }
}

async fn agent_attempt<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    agent: &mut Agent,
    key: &PublicKey,
    limit: Duration,
) -> Result<Outcome, SshError> {
    let hash = if key.algorithm().is_rsa() {
        rsa_hash(handle, limit).await?
    } else {
        None
    };
    // The agent signs inside this call, so its answer time is bounded together with the server's.
    match tokio::time::timeout(
        limit,
        handle.authenticate_publickey_with(user, key.clone(), hash, agent),
    )
    .await
    {
        Err(_) => Err(SshError::Timeout),
        Ok(Ok(result)) => Ok(outcome_of(result)),
        // The agent refused or failed to sign: this identity is out, the connection is fine.
        Ok(Err(_)) => Ok(Outcome::Skipped),
    }
}

async fn key_attempt<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    key: PrivateKey,
    limit: Duration,
) -> Result<Outcome, SshError> {
    let hash = if key.algorithm().is_rsa() {
        rsa_hash(handle, limit).await?
    } else {
        None
    };
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    Ok(outcome_of(
        bounded(limit, handle.authenticate_publickey(user, key)).await?,
    ))
}

/// Probes the server's methods with `none`, then works through the planner's attempts.
async fn authenticate<H: client::Handler, I: Interaction>(
    handle: &mut client::Handle<H>,
    params: &ConnectParams,
    interaction: &I,
) -> Result<(), SshError> {
    let user = match &params.username {
        Some(u) => u.clone(),
        None => interaction
            .username(&params.host)
            .await
            .ok_or_else(auth_failed)?,
    };
    // The host key was accepted during the handshake, so nothing secret has been sent yet; `none`
    // only asks which methods the server offers.
    let limit = params.connect_timeout;
    let mut offered = match bounded(limit, handle.authenticate_none(&user)).await? {
        russh::client::AuthResult::Success => return Ok(()),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => offered_from(&remaining_methods),
    };
    let stored_password = match &params.credential {
        Some(Credential::Password(p)) => Some(p),
        _ => None,
    };
    let (key_text, stored_passphrase) = match &params.credential {
        Some(Credential::PrivateKey { key, passphrase }) => (Some(key), passphrase.as_ref()),
        _ => (None, None),
    };

    // The credential key, when it decodes without asking, goes first among the agent's identities
    // (the user's explicit choice); an encrypted key without stored passphrase is only decoded at
    // its own attempt, after the agent had its chance.
    let mut key: Option<PrivateKey> = key_text.and_then(|t| decode_key(t, stored_passphrase).ok());
    let preferred = key.as_ref().map(|k| k.public_key().clone());
    let mut agent = None;
    let mut identities = Vec::new();
    if offered.publickey
        && let Some(mut connected) = tokio::time::timeout(limit, agent::connect())
            .await
            .ok()
            .flatten()
    {
        identities = agent::order_identities(
            agent_identities(&mut connected, limit).await,
            preferred.as_ref(),
        );
        agent = Some(connected);
    }

    let mut planner = AuthPlanner::new(Available {
        agent_identities: identities.len(),
        has_key: key_text.is_some(),
        has_password: stored_password.is_some(),
    });
    let mut key_unreadable = false;
    while let Some(attempt) = planner.next(offered) {
        let outcome = match attempt {
            Attempt::Agent(i) => match (agent.as_mut(), identities.get(i)) {
                (Some(agent), Some(identity)) => {
                    agent_attempt(handle, &user, agent, identity, limit).await?
                }
                _ => Outcome::Skipped,
            },
            Attempt::CredentialKey => {
                let loaded = match (key.take(), key_text) {
                    (Some(already), _) => Some(already),
                    (None, Some(text)) => load_key(text, stored_passphrase, interaction).await,
                    (None, None) => None,
                };
                match loaded {
                    Some(k) => key_attempt(handle, &user, k, limit).await?,
                    None => {
                        key_unreadable = true;
                        Outcome::Skipped
                    }
                }
            }
            Attempt::CredentialPassword => match stored_password {
                Some(password) => password_attempt(handle, &user, password, limit).await?,
                None => Outcome::Skipped,
            },
            // A cancelled prompt ends authentication, but the final error must still say that the
            // key was unreadable.
            Attempt::KeyboardInteractive => {
                keyboard_interactive_attempt(handle, &user, interaction, limit)
                    .await
                    .map_err(|e| with_key_flag(e, key_unreadable))?
            }
            Attempt::TypedPassword => {
                let password = interaction
                    .password(&user, &params.host)
                    .await
                    .ok_or(SshError::AuthFailed { key_unreadable })?;
                password_attempt(handle, &user, &password, limit).await?
            }
        };
        match outcome {
            Outcome::Success => return Ok(()),
            Outcome::Rejected(next) => offered = next,
            Outcome::Skipped => {}
        }
    }
    Err(SshError::AuthFailed { key_unreadable })
}

/// How often the handshake deadline is checked.
const TICK: Duration = Duration::from_millis(50);

/// Runs the handshake, failing with `Timeout` once `limit` of waiting for the server has passed.
/// Time the handler spends on a user decision (`prompting` set) does not count.
async fn with_handshake_deadline<T>(
    fut: impl Future<Output = T>,
    prompting: &AtomicBool,
    limit: Duration,
) -> Result<T, SshError> {
    tokio::pin!(fut);
    let mut spent = Duration::ZERO;
    loop {
        tokio::select! {
            out = &mut fut => return Ok(out),
            () = tokio::time::sleep(TICK) => {
                if !prompting.load(Ordering::SeqCst) {
                    spent += TICK;
                    if spent >= limit {
                        return Err(SshError::Timeout);
                    }
                }
            }
        }
    }
}

impl Session {
    /// Connects, checks the host key, authenticates and starts a shell on a PTY.
    pub async fn connect<I: Interaction>(
        params: ConnectParams,
        interaction: Arc<I>,
    ) -> Result<(Session, mpsc::Receiver<SessionEvent>), SshError> {
        let known_hosts = KnownHosts::new(&params.known_hosts);
        let mut config = client_config(params.keepalive_secs);
        let stored = known_hosts.stored_algorithms(&params.host, params.port)?;
        restrict_host_key_algorithms(&mut config, &stored);

        // Name resolution and the TCP connect get the whole timeout.
        let stream = match tokio::time::timeout(
            params.connect_timeout,
            TcpStream::connect((params.host.as_str(), params.port)),
        )
        .await
        {
            Err(_) => return Err(SshError::Timeout),
            Ok(Err(_)) => return Err(SshError::ConnectFailed),
            Ok(Ok(stream)) => stream,
        };
        let _ = stream.set_nodelay(true);

        let rejection: Rejection = Rejection::default();
        let prompting = Arc::new(AtomicBool::new(false));
        let handler = ClientHandler {
            host: params.host.clone(),
            port: params.port,
            known_hosts,
            interaction: Arc::clone(&interaction),
            rejection: Arc::clone(&rejection),
            prompting: Arc::clone(&prompting),
        };
        let handshake = client::connect_stream(Arc::new(config), stream, handler);
        let mut handle =
            match with_handshake_deadline(handshake, &prompting, params.connect_timeout).await? {
                Ok(handle) => handle,
                Err(e) => {
                    let parked = rejection.lock().ok().and_then(|mut slot| slot.take());
                    return Err(parked.unwrap_or(match e {
                        russh::Error::IO(_) | russh::Error::ConnectionTimeout => {
                            SshError::ConnectFailed
                        }
                        _ => SshError::Protocol,
                    }));
                }
            };

        authenticate(&mut handle, &params, interaction.as_ref()).await?;

        let channel =
            match tokio::time::timeout(params.connect_timeout, handle.channel_open_session()).await
            {
                Err(_) => return Err(SshError::Timeout),
                Ok(Err(_)) => return Err(SshError::Protocol),
                Ok(Ok(channel)) => channel,
            };
        let (mut reader, writer) = channel.split();
        writer
            .request_pty(true, "xterm-256color", params.cols, params.rows, 0, 0, &[])
            .await
            .map_err(io_to_lost)?;
        let pty_reply = async {
            loop {
                match reader.wait().await {
                    Some(ChannelMsg::Success) => return Ok(()),
                    Some(ChannelMsg::Failure) | None => return Err(SshError::Protocol),
                    Some(_) => {}
                }
            }
        };
        tokio::time::timeout(params.connect_timeout, pty_reply)
            .await
            .map_err(|_| SshError::Timeout)??;
        writer.request_shell(false).await.map_err(io_to_lost)?;

        let (tx, rx) = mpsc::channel(EVENT_QUEUE);
        let stop = Arc::new(Notify::new());
        let closed = Arc::new(AtomicBool::new(false));
        tokio::spawn(pump(
            handle,
            reader,
            tx,
            Arc::clone(&stop),
            Arc::clone(&closed),
        ));
        Ok((
            Session {
                writer,
                stop,
                closed,
            },
            rx,
        ))
    }

    pub async fn write(&self, data: Vec<u8>) -> Result<(), SshError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SshError::Closed);
        }
        self.writer.data_bytes(data).await.map_err(|_| self.gone())
    }

    pub async fn resize(&self, cols: u32, rows: u32) -> Result<(), SshError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SshError::Closed);
        }
        self.writer
            .window_change(cols, rows, 0, 0)
            .await
            .map_err(|_| self.gone())
    }

    /// Ends the session; the events channel then yields `Closed(ClosedByUser)`.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.stop.notify_one();
    }

    fn gone(&self) -> SshError {
        if self.closed.load(Ordering::SeqCst) {
            SshError::Closed
        } else {
            SshError::ConnectionLost
        }
    }
}

impl Drop for Session {
    /// A dropped handle ends the connection even when the consumer keeps the receiver (it then
    /// still gets `Closed(ClosedByUser)`). `close` only sets a flag and stores a wake-up permit, so
    /// it never blocks and calling it again after an explicit `close` is harmless.
    fn drop(&mut self) {
        self.close();
    }
}

enum Flow {
    Continue,
    UserClose,
    ReceiverGone,
}

/// Hands one event to the consumer. Blocks while the queue is full, which stops reading from the
/// channel and so, through russh's own bounded channel and the SSH window, the server.
async fn deliver(tx: &mpsc::Sender<SessionEvent>, stop: &Notify, event: SessionEvent) -> Flow {
    tokio::select! {
        sent = tx.send(event) => if sent.is_ok() { Flow::Continue } else { Flow::ReceiverGone },
        () = stop.notified() => Flow::UserClose,
    }
}

async fn pump<H: client::Handler>(
    handle: client::Handle<H>,
    mut reader: russh::ChannelReadHalf,
    tx: mpsc::Sender<SessionEvent>,
    stop: Arc<Notify>,
    closed: Arc<AtomicBool>,
) {
    let mut exit: Option<u32> = None;
    let mut exited = false;
    let reason = loop {
        let msg = tokio::select! {
            biased;
            () = stop.notified() => break CloseReason::ClosedByUser,
            msg = reader.wait() => msg,
        };
        let data = match msg {
            Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => data,
            Some(ChannelMsg::ExitStatus { exit_status }) => {
                exit = Some(exit_status);
                exited = true;
                continue;
            }
            Some(ChannelMsg::ExitSignal { .. }) => {
                exited = true;
                continue;
            }
            Some(ChannelMsg::Close) => break CloseReason::Exit(exit),
            None if exited => break CloseReason::Exit(exit),
            None => break CloseReason::ConnectionLost,
            Some(_) => continue,
        };
        match deliver(&tx, &stop, SessionEvent::Output(data.to_vec())).await {
            Flow::Continue => {}
            Flow::UserClose => break CloseReason::ClosedByUser,
            Flow::ReceiverGone => {
                closed.store(true, Ordering::SeqCst);
                // Nobody listens any more: just end the connection.
                drop(reader);
                let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
                return;
            }
        }
    };
    closed.store(true, Ordering::SeqCst);
    // Release russh's channel first: its session loop may be parked on a full channel.
    drop(reader);
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
    let _ = tx.send(SessionEvent::Closed(reason)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction::{
        HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt,
    };
    use russh::client::Prompt;
    use std::collections::VecDeque;

    /// Answers every keyboard-interactive prompt with "x"; nothing else is asked in these tests.
    struct Answering;

    impl Interaction for Answering {
        async fn host_key(&self, _: HostKeyPrompt) -> HostKeyDecision {
            HostKeyDecision::Reject
        }
        async fn host_key_changed(&self, _: HostKeyChangedPrompt) -> HostKeyChangedDecision {
            HostKeyChangedDecision::Reject
        }
        async fn keyboard_interactive(&self, round: KiRound) -> Option<Vec<SecretString>> {
            Some(
                round
                    .prompts
                    .iter()
                    .map(|_| SecretString::from("x"))
                    .collect(),
            )
        }
        async fn password(&self, _: &str, _: &str) -> Option<SecretString> {
            None
        }
        async fn username(&self, _: &str) -> Option<String> {
            None
        }
        async fn key_passphrase(&self) -> Option<SecretString> {
            None
        }
    }

    /// Replies from a queue and counts the answers it received.
    struct Scripted {
        replies: VecDeque<KeyboardInteractiveAuthResponse>,
        calls: usize,
    }

    impl KiTransport for Scripted {
        async fn respond(
            &mut self,
            _answers: Vec<String>,
        ) -> Result<KeyboardInteractiveAuthResponse, SshError> {
            self.calls += 1;
            Ok(self.replies.pop_front().expect("more replies than rounds"))
        }
    }

    fn info() -> KeyboardInteractiveAuthResponse {
        KeyboardInteractiveAuthResponse::InfoRequest {
            name: String::new(),
            instructions: String::new(),
            prompts: vec![Prompt {
                prompt: "Password: ".into(),
                echo: false,
            }],
        }
    }

    /// The server asks `rounds` times in total, then (if `then_success`) accepts.
    async fn run(rounds: usize, then_success: bool) -> (Result<Outcome, SshError>, usize) {
        let mut replies: VecDeque<_> = (1..rounds).map(|_| info()).collect();
        if then_success {
            replies.push_back(KeyboardInteractiveAuthResponse::Success);
        }
        let mut transport = Scripted { replies, calls: 0 };
        let out = keyboard_interactive_rounds(info(), &mut transport, &Answering).await;
        (out, transport.calls)
    }

    #[tokio::test]
    async fn keyboard_interactive_accepts_a_success_after_the_last_allowed_round() {
        // The reply to the 5th answer is Success: it must be honoured, not dropped with the loop.
        let (out, calls) = run(MAX_KI_ROUNDS, true).await;
        assert!(matches!(out, Ok(Outcome::Success)), "{:?}", out.map(|_| ()));
        assert_eq!(calls, MAX_KI_ROUNDS);
    }

    #[tokio::test]
    async fn keyboard_interactive_gives_up_after_too_many_rounds() {
        // A 6th request is refused: MAX_KI_ROUNDS answers were sent, no more.
        let (out, calls) = run(MAX_KI_ROUNDS + 1, false).await;
        assert!(matches!(out, Ok(Outcome::Rejected(_))));
        assert_eq!(calls, MAX_KI_ROUNDS);
    }

    #[test]
    fn client_config_sets_keepalive_or_none() {
        let c = client_config(30);
        assert_eq!(c.keepalive_interval, Some(Duration::from_secs(30)));
        assert_eq!(c.keepalive_max, 3);
        assert_eq!(client_config(0).keepalive_interval, None);
    }

    /// The offered algorithms are pinned: a russh upgrade that adds a weak one fails here and has
    /// to be looked at (Docs/DECISIONS.md, "M6 security review: SSH algorithm policy").
    #[test]
    fn client_config_offers_no_weak_algorithms() {
        let p = client_config(30).preferred;
        let names = |list: Vec<&str>| list.join(",");
        let kex = names(p.kex.iter().map(AsRef::as_ref).collect());
        let cipher = names(p.cipher.iter().map(AsRef::as_ref).collect());
        let mac = names(p.mac.iter().map(AsRef::as_ref).collect());
        for weak in [
            "sha1",
            "group1-",
            "group14-sha1",
            "diffie-hellman-group-exchange-sha1",
        ] {
            assert!(!kex.contains(weak), "kex {kex} offers {weak}");
        }
        for weak in ["cbc", "3des", "arcfour", "none"] {
            assert!(!cipher.contains(weak), "cipher {cipher} offers {weak}");
        }
        assert!(!mac.contains("sha1") && !mac.contains("md5"), "mac {mac}");
        // plain `ssh-rsa` (an RSA signature over SHA-1) is no longer offered by OpenSSH either
        assert!(
            !p.key.contains(&Algorithm::Rsa { hash: None }),
            "host key algorithms: {:?}",
            p.key
        );
        for wanted in [
            Algorithm::Ed25519,
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha256),
            },
            Algorithm::Rsa {
                hash: Some(HashAlg::Sha512),
            },
        ] {
            assert!(p.key.contains(&wanted), "{wanted:?} missing");
        }
    }

    #[test]
    fn client_config_bounds_the_data_in_flight() {
        let c = client_config(30);
        assert_eq!(c.window_size, 512 * 1024);
        assert_eq!(c.channel_buffer_size, 4);
        assert_eq!(c.maximum_packet_size, 32 * 1024);
        assert_eq!(EVENT_QUEUE, 4);
    }
}
