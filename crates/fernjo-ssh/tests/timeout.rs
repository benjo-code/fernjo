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
