//! One SSH connection with an interactive shell on a PTY.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::client::{self, KeyboardInteractiveAuthResponse, Msg};
use russh::keys::Algorithm;
use russh::keys::ssh_key::HashAlg;
use russh::{ChannelMsg, ChannelWriteHalf, Disconnect};
use secrecy::{ExposeSecret, SecretString};
use tokio::net::TcpStream;
use tokio::sync::{Notify, mpsc};

use crate::SshError;
use crate::auth_plan::{Attempt, AuthPlanner, Available, Offered};
use crate::handler::{ClientHandler, Rejection};
use crate::interaction::{Interaction, KiPrompt, KiRound};
use crate::known_hosts::KnownHosts;
use crate::params::{ConnectParams, Credential};

/// Entries the events channel holds before the session task blocks (backpressure towards the server).
const EVENT_QUEUE: usize = 64;
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
    client::Config {
        keepalive_interval: (keepalive_secs > 0)
            .then(|| Duration::from_secs(u64::from(keepalive_secs))),
        keepalive_max: 3,
        nodelay: true,
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
}

fn auth_failed() -> SshError {
    SshError::AuthFailed {
        key_unreadable: false,
    }
}

fn io_to_lost(_: russh::Error) -> SshError {
    SshError::ConnectionLost
}

async fn password_attempt<H: client::Handler>(
    handle: &mut client::Handle<H>,
    user: &str,
    password: &SecretString,
) -> Result<Outcome, SshError> {
    match handle
        .authenticate_password(user, password.expose_secret().to_owned())
        .await
        .map_err(io_to_lost)?
    {
        russh::client::AuthResult::Success => Ok(Outcome::Success),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => Ok(Outcome::Rejected(offered_from(&remaining_methods))),
    }
}

async fn keyboard_interactive_attempt<H: client::Handler, I: Interaction>(
    handle: &mut client::Handle<H>,
    user: &str,
    interaction: &I,
) -> Result<Outcome, SshError> {
    let mut response = handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
        .map_err(io_to_lost)?;
    for _ in 0..MAX_KI_ROUNDS {
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
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await
                    .map_err(io_to_lost)?;
            }
        }
    }
    Ok(Outcome::Rejected(Offered {
        publickey: false,
        password: false,
        keyboard_interactive: false,
    }))
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
    let mut offered = match handle.authenticate_none(&user).await.map_err(io_to_lost)? {
        russh::client::AuthResult::Success => return Ok(()),
        russh::client::AuthResult::Failure {
            remaining_methods, ..
        } => offered_from(&remaining_methods),
    };
    let stored_password = match &params.credential {
        Some(Credential::Password(p)) => Some(p),
        _ => None,
    };
    // Task 5 adds agent identities and the private key here and handles their attempts below.
    let mut planner = AuthPlanner::new(Available {
        agent_identities: 0,
        has_key: false,
        has_password: stored_password.is_some(),
    });
    while let Some(attempt) = planner.next(offered) {
        let outcome = match attempt {
            Attempt::CredentialPassword => match stored_password {
                Some(password) => password_attempt(handle, &user, password).await?,
                None => continue,
            },
            Attempt::KeyboardInteractive => {
                keyboard_interactive_attempt(handle, &user, interaction).await?
            }
            Attempt::TypedPassword => {
                let password = interaction
                    .password(&user, &params.host)
                    .await
                    .ok_or_else(auth_failed)?;
                password_attempt(handle, &user, &password).await?
            }
            // Not planned yet (see above).
            Attempt::Agent(_) | Attempt::CredentialKey => continue,
        };
        match outcome {
            Outcome::Success => return Ok(()),
            Outcome::Rejected(next) => offered = next,
        }
    }
    Err(auth_failed())
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

        // The timeout covers name resolution and the TCP connect only: the host key prompt that
        // follows waits for the user for as long as it takes.
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
        let handler = ClientHandler {
            host: params.host.clone(),
            port: params.port,
            known_hosts,
            interaction: Arc::clone(&interaction),
            rejection: Arc::clone(&rejection),
        };
        let mut handle = match client::connect_stream(Arc::new(config), stream, handler).await {
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

        let channel = handle
            .channel_open_session()
            .await
            .map_err(|_| SshError::Protocol)?;
        let (mut reader, writer) = channel.split();
        writer
            .request_pty(true, "xterm-256color", params.cols, params.rows, 0, 0, &[])
            .await
            .map_err(io_to_lost)?;
        loop {
            match reader.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) | None => return Err(SshError::Protocol),
                Some(_) => {}
            }
        }
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

    #[test]
    fn client_config_sets_keepalive_or_none() {
        let c = client_config(30);
        assert_eq!(c.keepalive_interval, Some(Duration::from_secs(30)));
        assert_eq!(c.keepalive_max, 3);
        assert_eq!(client_config(0).keepalive_interval, None);
    }
}
