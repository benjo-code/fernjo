//! Sessions against an in-process russh server that misbehaves after login: it opens channels of
//! its own, or stops reading its socket. Needs no SSH test server (runs everywhere, CI included).

mod support;

use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use fernjo_ssh::{CloseReason, ConnectParams, Credential, HostKeyDecision, Session, SessionEvent};
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};
use russh::keys::{PrivateKey, ssh_key};
use russh::server::{self, Auth, Msg, Session as ServerSession};
use russh::{Channel, ChannelId, ChannelOpenFailure, MethodKind, MethodSet};
use secrecy::SecretString;
use support::ScriptedInteraction;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(10);

/// A fixed host key: the tests trust it once, so its value does not matter.
fn host_key() -> PrivateKey {
    let pair = Ed25519Keypair::from_seed(&[7; 32]);
    ssh_key::PrivateKey::new(KeypairData::Ed25519(pair), "test host").unwrap()
}

/// Stops and resumes the server's reads.
#[derive(Default)]
struct Pause {
    paused: AtomicBool,
    waker: Mutex<Option<Waker>>,
}

impl Pause {
    fn stop(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        if let Some(waker) = self.waker.lock().unwrap().take() {
            waker.wake();
        }
    }
}

/// The server's TCP stream; while paused it reads nothing (a peer that stops reading while
/// keeping the connection open: the client's writes fill the socket buffers and stall).
struct Pausable {
    inner: TcpStream,
    pause: Arc<Pause>,
}

impl AsyncRead for Pausable {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.pause.paused.load(Ordering::SeqCst) {
            *self.pause.waker.lock().unwrap() = Some(cx.waker().clone());
            // checked again: a resume between the first check and storing the waker
            if self.pause.paused.load(Ordering::SeqCst) {
                return Poll::Pending;
            }
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for Pausable {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Accepts the password login, one session channel with a PTY and a shell, and echoes input.
/// Reports its handle once the shell runs.
struct Server {
    shell: Option<oneshot::Sender<server::Handle>>,
}

impl server::Handler for Server {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.channel_success(channel)?;
        if let Some(tx) = self.shell.take() {
            let _ = tx.send(session.handle());
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut ServerSession,
    ) -> Result<(), Self::Error> {
        session.data(channel, data.to_vec())
    }
}

struct Running {
    session: Arc<Session>,
    rx: mpsc::Receiver<SessionEvent>,
    server: server::Handle,
    pause: Arc<Pause>,
    /// Fires when the server's side of the connection ended.
    server_ended: oneshot::Receiver<()>,
}

/// Starts the server on a local port and connects a session to it.
async fn connect() -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let pause = Arc::new(Pause::default());
    let (shell_tx, shell_rx) = oneshot::channel();
    let (ended_tx, server_ended) = oneshot::channel();
    let server_pause = Arc::clone(&pause);
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let config = Arc::new(server::Config {
            methods: MethodSet::from(&[MethodKind::Password][..]),
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            keys: vec![host_key()],
            // A window far beyond the socket buffers, so the client stalls on the socket, not on
            // the SSH window.
            window_size: 1 << 30,
            ..server::Config::default()
        });
        let stream = Pausable {
            inner: stream,
            pause: server_pause,
        };
        let running = server::run_stream(
            config,
            stream,
            Server {
                shell: Some(shell_tx),
            },
        )
        .await
        .unwrap();
        let _ = running.await;
        let _ = ended_tx.send(());
    });

    let dir = tempfile::tempdir().unwrap();
    let mut params = ConnectParams::new("127.0.0.1", port, dir.path().join("kh"));
    params.username = Some("u".into());
    params.credential = Some(Credential::Password(SecretString::from("p")));
    let interaction = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, rx) = timeout(WAIT, Session::connect(params, interaction))
        .await
        .expect("connect timed out")
        .expect("connect failed");
    let server = timeout(WAIT, shell_rx)
        .await
        .expect("the shell never started")
        .unwrap();
    Running {
        session: Arc::new(session),
        rx,
        server,
        pause,
        server_ended,
    }
}

/// Writes `text` and waits until the server's echo of it arrives.
async fn echo(running: &mut Running, text: &str) {
    running
        .session
        .write(text.as_bytes().to_vec())
        .await
        .expect("write failed");
    timeout(WAIT, async {
        let mut seen = String::new();
        while !seen.contains(text) {
            match running.rx.recv().await {
                Some(SessionEvent::Output(b)) => seen.push_str(&String::from_utf8_lossy(&b)),
                Some(SessionEvent::Closed(r)) => panic!("closed ({r:?}) while waiting for {text}"),
                None => panic!("events ended while waiting for {text}"),
            }
        }
    })
    .await
    .expect("no echo");
}

/// M11 review #23: fernjo requests no forwarding, so every channel the server opens is refused
/// and the session goes on as before.
#[tokio::test]
async fn server_opened_channels_are_refused_and_the_session_keeps_working() {
    let mut running = connect().await;
    echo(&mut running, "before").await;
    let server = running.server.clone();
    let opens: Vec<(&str, Result<Channel<Msg>, russh::Error>)> = vec![
        (
            "forwarded-tcpip",
            server
                .channel_open_forwarded_tcpip("127.0.0.1", 8080, "203.0.113.5", 40000)
                .await,
        ),
        ("session", server.channel_open_session().await),
        (
            "direct-tcpip",
            server
                .channel_open_direct_tcpip("127.0.0.1", 22, "203.0.113.5", 40001)
                .await,
        ),
        ("x11", server.channel_open_x11("203.0.113.5", 6000).await),
        ("agent", server.channel_open_agent().await),
        (
            "forwarded-streamlocal",
            server
                .channel_open_forwarded_streamlocal("/tmp/socket")
                .await,
        ),
        (
            "direct-streamlocal",
            server.channel_open_direct_streamlocal("/tmp/socket").await,
        ),
    ];
    for (kind, result) in opens {
        assert!(
            matches!(
                result,
                Err(russh::Error::ChannelOpenFailure(
                    ChannelOpenFailure::AdministrativelyProhibited
                ))
            ),
            "{kind}: expected a refusal, got {:?}",
            result.map(|c| c.id())
        );
    }
    echo(&mut running, "after").await;
}

/// M11 review #22: a server that stops reading stalls the client's writes in the socket. Closing
/// the session must still end it within a bound: the close event arrives and a write that hangs
/// in the stalled connection returns, so neither the connection nor the tasks are left behind.
#[tokio::test]
async fn closing_ends_a_session_whose_server_stopped_reading() {
    let mut running = connect().await;
    echo(&mut running, "up").await;
    running.pause.stop();

    // Write until a write no longer returns: the socket buffers are full.
    let written = Arc::new(AtomicUsize::new(0));
    let writer = {
        let (session, written) = (Arc::clone(&running.session), Arc::clone(&written));
        tokio::spawn(async move {
            let chunk = vec![b'x'; 64 * 1024];
            loop {
                if session.write(chunk.clone()).await.is_err() {
                    return;
                }
                written.fetch_add(chunk.len(), Ordering::SeqCst);
            }
        })
    };
    let mut last = 0;
    let mut still_since = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let now = written.load(Ordering::SeqCst);
        if now != last {
            last = now;
            still_since = Instant::now();
        } else if still_since.elapsed() >= Duration::from_secs(1) {
            break;
        }
        assert!(
            now < 512 * 1024 * 1024,
            "the writes never stalled ({now} bytes)"
        );
        // drain output so only the socket can hold the writer back
        while running.rx.try_recv().is_ok() {}
    }
    assert!(!writer.is_finished(), "the writer ended before the close");

    running.session.close();
    let started = Instant::now();
    let reason = timeout(WAIT, async {
        loop {
            match running.rx.recv().await {
                Some(SessionEvent::Closed(r)) => return Some(r),
                Some(SessionEvent::Output(_)) => {}
                None => return None,
            }
        }
    })
    .await
    .expect("no close event: the session hangs in the stalled connection");
    assert!(
        matches!(reason, Some(CloseReason::ClosedByUser)),
        "{reason:?}"
    );
    timeout(WAIT, writer)
        .await
        .expect("the stalled write never returned")
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "closing took {:?}",
        started.elapsed()
    );
    assert_connection_closed(&running.pause, running.server_ended).await;
    // the session handle lived until here, so nothing but the close ended the connection
    drop(running.session);
}

/// M11 review #22: dropping the events receiver (the app aborts the task that reads it when a
/// tab closes) ends the session too, even while a write hangs in the stalled connection.
#[tokio::test]
async fn dropping_the_receiver_ends_a_stalled_session() {
    let mut running = connect().await;
    echo(&mut running, "up").await;
    running.pause.stop();
    let writer = {
        let session = Arc::clone(&running.session);
        tokio::spawn(async move {
            let chunk = vec![b'x'; 64 * 1024];
            while session.write(chunk.clone()).await.is_ok() {}
        })
    };
    // give the writes time to fill the socket buffers
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!writer.is_finished(), "the writer ended before the drop");
    drop(running.rx);
    timeout(WAIT, writer)
        .await
        .expect("the stalled write never returned after the receiver was dropped")
        .unwrap();
    assert_connection_closed(&running.pause, running.server_ended).await;
    // the session handle lived until here, so nothing but the close ended the connection
    drop(running.session);
}

/// The client really closed its socket: once the server reads again it drains what was buffered
/// and then finds the connection ended. A client whose connection task still hung in the write
/// (the socket open, the task alive) would instead go on talking to it.
async fn assert_connection_closed(pause: &Pause, server_ended: oneshot::Receiver<()>) {
    pause.resume();
    timeout(WAIT, server_ended)
        .await
        .expect("the client's connection is still open")
        .unwrap();
}
