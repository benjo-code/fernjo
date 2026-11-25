//! Connect timeout against local fake servers; needs no SSH server.

mod support;

use std::sync::Arc;
use std::time::Duration;

use fernjo_ssh::{ConnectParams, Session};
use support::ScriptedInteraction;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::Instant;

async fn connect_to_silent_server(banner: Option<&'static [u8]>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        if let Some(b) = banner {
            stream.write_all(b).await.unwrap();
        }
        // Hold the connection open and say nothing more.
        tokio::time::sleep(Duration::from_secs(30)).await;
    });

    let dir = tempfile::tempdir().unwrap();
    let mut params = ConnectParams::new("127.0.0.1", port, dir.path().join("kh"));
    params.username = Some("u".into());
    params.connect_timeout = Duration::from_millis(300);
    let started = Instant::now();
    let Err(e) = Session::connect(params, Arc::new(ScriptedInteraction::new())).await else {
        panic!("connect must fail");
    };
    assert_eq!(e.code(), "timeout");
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "took {:?}",
        started.elapsed()
    );
    server.abort();
}

#[tokio::test]
async fn silent_server_that_accepts_tcp_ends_with_timeout() {
    connect_to_silent_server(None).await;
}

#[tokio::test]
async fn server_that_sends_a_banner_and_goes_silent_ends_with_timeout() {
    connect_to_silent_server(Some(b"SSH-2.0-fake\r\n")).await;
}

/// A server that accepts, sends its banner and then only reads. Reports when it saw the client's
/// side of the connection closed (EOF or an error).
async fn banner_then_read_until_closed() -> (u16, tokio::sync::oneshot::Receiver<Instant>) {
    use tokio::io::AsyncReadExt;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        stream.write_all(b"SSH-2.0-fake\r\n").await.unwrap();
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = closed_tx.send(Instant::now());
    });
    (port, closed_rx)
}

fn silent_params(port: u16, dir: &std::path::Path) -> ConnectParams {
    let mut params = ConnectParams::new("127.0.0.1", port, dir.join("kh"));
    params.username = Some("u".into());
    params.connect_timeout = Duration::from_millis(300);
    params
}

#[tokio::test]
async fn a_pre_key_exchange_timeout_leaves_no_session_task() {
    let (port, closed) = banner_then_read_until_closed().await;
    let dir = tempfile::tempdir().unwrap();
    let Err(e) = Session::connect(
        silent_params(port, dir.path()),
        Arc::new(ScriptedInteraction::new()),
    )
    .await
    else {
        panic!("connect must fail");
    };
    assert_eq!(e.code(), "timeout");
    let failed_at = Instant::now();
    let closed_at = tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .expect("the server must see the connection closed within a second of the timeout")
        .unwrap();
    assert!(closed_at.duration_since(failed_at) < Duration::from_secs(1));
}

#[tokio::test]
async fn a_cancelled_handshake_leaves_no_session_task() {
    let (port, closed) = banner_then_read_until_closed().await;
    let dir = tempfile::tempdir().unwrap();
    let mut params = silent_params(port, dir.path());
    params.connect_timeout = Duration::from_secs(30);
    let connect = tokio::spawn(async move {
        let _ = Session::connect(params, Arc::new(ScriptedInteraction::new())).await;
    });
    // the banner has arrived and the key exchange waits for the silent server
    tokio::time::sleep(Duration::from_millis(300)).await;
    connect.abort();
    let _ = connect.await;
    tokio::time::timeout(Duration::from_secs(1), closed)
        .await
        .expect("the server must see the connection closed within a second of the cancel")
        .unwrap();
}
