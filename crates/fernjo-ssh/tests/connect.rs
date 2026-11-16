//! Integration tests against the OpenSSH test server (`tools/ssh-test-server.sh`). Each test skips
//! when `FERNJO_TEST_SSH_ADDR` is unset; with `CI` set the missing variable is an error.

mod support;

use std::sync::Arc;
use std::time::Duration;

use fernjo_ssh::{
    CloseReason, ConnectParams, Credential, HostKeyChangedDecision, HostKeyDecision, Session,
    SessionEvent, SshError,
};
use secrecy::SecretString;
use support::{ScriptedInteraction, TestServer};
use tokio::sync::mpsc::Receiver;
use tokio::time::{Instant, timeout};

fn params(s: &TestServer, known_hosts: &std::path::Path) -> ConnectParams {
    let mut p = ConnectParams::new(&s.host, s.port, known_hosts.to_path_buf());
    p.username = Some(s.user.clone());
    p.credential = Some(Credential::Password(SecretString::from(s.password.clone())));
    p
}

/// Reads output until it contains `needle`; panics after `limit`.
async fn read_until(rx: &mut Receiver<SessionEvent>, needle: &str, limit: Duration) -> String {
    let deadline = Instant::now() + limit;
    let mut seen = String::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match timeout(left, rx.recv()).await {
            Ok(Some(SessionEvent::Output(b))) => {
                seen.push_str(&String::from_utf8_lossy(&b));
                if seen.contains(needle) {
                    return seen;
                }
            }
            Ok(Some(SessionEvent::Closed(r))) => panic!("closed ({r:?}) before {needle:?}"),
            Ok(None) => panic!("events channel ended before {needle:?}"),
            Err(_) => panic!("timeout waiting for {needle:?}"),
        }
    }
}

const WAIT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn password_login_runs_a_command() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, mut rx) = Session::connect(params(&s, &dir.path().join("kh")), ia)
        .await
        .unwrap();
    session
        .write(b"echo fernjo$((20+22))\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, "fernjo42", WAIT).await;
    session.close();
}

#[tokio::test]
async fn unknown_host_trust_always_writes_known_hosts_and_second_connect_does_not_prompt() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let first = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustAlways));
    let (session, _rx) = Session::connect(params(&s, &kh), Arc::clone(&first))
        .await
        .unwrap();
    session.close();
    assert_eq!(first.host_key_calls(), 1);
    let prompt = &first.host_key_prompts()[0];
    assert_eq!(
        (prompt.host.as_str(), prompt.port),
        (s.host.as_str(), s.port)
    );
    assert!(prompt.fingerprint.starts_with("SHA256:"));
    let text = std::fs::read_to_string(&kh).unwrap();
    assert!(
        text.starts_with(&format!("[{}]:{} ", s.host, s.port)),
        "{text}"
    );

    // An empty script would answer Reject, so a prompt here would fail the connect.
    let second = Arc::new(ScriptedInteraction::new());
    let (session, _rx) = Session::connect(params(&s, &kh), Arc::clone(&second))
        .await
        .unwrap();
    session.close();
    assert_eq!(second.host_key_calls(), 0);
}

#[tokio::test]
async fn trust_once_does_not_write_known_hosts() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, _rx) = Session::connect(params(&s, &kh), ia).await.unwrap();
    session.close();
    assert!(!kh.exists());
}

#[tokio::test]
async fn rejecting_an_unknown_host_ends_with_host_key_rejected() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::Reject));
    let Err(e) = Session::connect(params(&s, &kh), Arc::clone(&ia)).await else {
        panic!("connect must fail");
    };
    assert_eq!(e.code(), "host_key_rejected");
    assert!(!kh.exists());
    assert_eq!(ia.password_calls() + ia.ki_calls() + ia.username_calls(), 0);
}

/// Writes a known_hosts line holding a different Ed25519 key for the test server; returns its
/// base64 blob.
fn write_foreign_key(s: &TestServer, kh: &std::path::Path) -> String {
    let mut pubfile = s.key_unauthorised.clone().into_os_string();
    pubfile.push(".pub");
    let line = std::fs::read_to_string(pubfile).unwrap();
    let mut fields = line.split_whitespace();
    let (kind, b64) = (fields.next().unwrap(), fields.next().unwrap());
    std::fs::write(kh, format!("[{}]:{} {kind} {b64}\n", s.host, s.port)).unwrap();
    b64.to_owned()
}

#[tokio::test]
async fn changed_host_key_is_blocked_before_any_authentication() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    write_foreign_key(&s, &kh);
    let before = std::fs::read_to_string(&kh).unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key_changed(HostKeyChangedDecision::Reject));
    let mut p = params(&s, &kh);
    p.username = None;
    let Err(e) = Session::connect(p, Arc::clone(&ia)).await else {
        panic!("connect must fail");
    };
    assert_eq!(e.code(), "host_key_changed_rejected");
    assert_eq!(ia.changed_calls(), 1);
    assert_eq!(ia.host_key_calls(), 0);
    assert_eq!(ia.password_calls(), 0);
    assert_eq!(ia.ki_calls(), 0);
    assert_eq!(ia.username_calls(), 0);
    assert_eq!(std::fs::read_to_string(&kh).unwrap(), before);
}

#[tokio::test]
async fn changed_host_key_replace_continues_and_rewrites_the_line() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let kh = dir.path().join("known_hosts");
    let old = write_foreign_key(&s, &kh);
    let ia = Arc::new(ScriptedInteraction::new().host_key_changed(HostKeyChangedDecision::Replace));
    let (session, _rx) = Session::connect(params(&s, &kh), Arc::clone(&ia))
        .await
        .unwrap();
    session.close();
    assert_eq!(ia.changed_calls(), 1);
    let text = std::fs::read_to_string(&kh).unwrap();
    assert!(!text.contains(&old), "old key still stored: {text}");
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.starts_with(&format!("[{}]:{} ssh-ed25519 ", s.host, s.port)));

    // The rewritten line is trusted without another prompt.
    let again = Arc::new(ScriptedInteraction::new());
    let (session, _rx) = Session::connect(params(&s, &kh), Arc::clone(&again))
        .await
        .unwrap();
    session.close();
    assert_eq!(again.changed_calls() + again.host_key_calls(), 0);
}

#[tokio::test]
async fn wrong_credentials_end_in_auth_failed_with_at_most_three_typed_password_prompts() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut ia = ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce);
    for _ in 0..4 {
        ia = ia
            .ki_answers(Some(vec!["wrong"]))
            .typed_password(Some("wrong"));
    }
    let ia = Arc::new(ia);
    let mut p = params(&s, &dir.path().join("kh"));
    p.credential = None;
    let Err(e) = Session::connect(p, Arc::clone(&ia)).await else {
        panic!("connect must fail");
    };
    assert!(matches!(e, SshError::AuthFailed { .. }), "{e:?}");
    assert!(
        (1..=3).contains(&ia.password_calls()),
        "{}",
        ia.password_calls()
    );
    assert!(ia.ki_calls() >= 1);
}

#[tokio::test]
async fn resize_reaches_the_server() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, mut rx) = Session::connect(params(&s, &dir.path().join("kh")), ia)
        .await
        .unwrap();
    session.resize(100, 30).await.unwrap();
    session.write(b"stty size\n".to_vec()).await.unwrap();
    read_until(&mut rx, "30 100", WAIT).await;
    session.close();
}

#[tokio::test]
async fn flood_blocks_when_receiver_is_slow() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, mut rx) = Session::connect(params(&s, &dir.path().join("kh")), ia)
        .await
        .unwrap();
    session.write(b"yes\n".to_vec()).await.unwrap();
    // Read nothing while the server floods.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let mut queued = 0usize;
    let mut entries = 0usize;
    while let Ok(ev) = rx.try_recv() {
        if let SessionEvent::Output(b) = ev {
            queued += b.len();
            entries += 1;
        }
    }
    eprintln!("flood: {queued} bytes in {entries} entries were queued after 2 s");
    assert!(queued < 16 * 1024 * 1024, "queued {queued} bytes");

    session.write(b"\x03".to_vec()).await.unwrap();
    session
        .write(b"echo mark$((40+2))\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, "mark42", Duration::from_secs(1)).await;
    session.close();
}

#[tokio::test]
async fn close_ends_with_closed_by_user() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, mut rx) = Session::connect(params(&s, &dir.path().join("kh")), ia)
        .await
        .unwrap();
    session.close();
    let end = timeout(WAIT, async {
        loop {
            match rx.recv().await {
                Some(SessionEvent::Closed(r)) => return Some(r),
                Some(_) => {}
                None => return None,
            }
        }
    })
    .await
    .expect("timeout");
    assert_eq!(end, Some(CloseReason::ClosedByUser));
    assert!(session.write(b"x".to_vec()).await.is_err());
}
