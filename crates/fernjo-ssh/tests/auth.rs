//! Key and keyboard-interactive authentication against the OpenSSH test server
//! (`tools/ssh-test-server.sh`); skipped without it, an error with `CI` set. The agent test lives
//! in `agent.rs` (its own process, see there).

mod support;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fernjo_ssh::{ConnectParams, Credential, HostKeyDecision, Session, SessionEvent, SshError};
use secrecy::SecretString;
use support::{ScriptedInteraction, TestServer};
use tokio::sync::mpsc::Receiver;
use tokio::time::{Instant, timeout};

fn params(s: &TestServer, known_hosts: &Path, credential: Option<Credential>) -> ConnectParams {
    let mut p = ConnectParams::new(&s.host, s.port, known_hosts.to_path_buf());
    p.username = Some(s.user.clone());
    p.credential = credential;
    p
}

fn key_credential(path: &Path, passphrase: Option<&str>) -> Credential {
    Credential::PrivateKey {
        key: SecretString::from(std::fs::read_to_string(path).unwrap()),
        passphrase: passphrase.map(SecretString::from),
    }
}

fn trusting() -> ScriptedInteraction {
    ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce)
}

const WAIT: Duration = Duration::from_secs(10);

/// Proves the shell works (and so that login succeeded), then closes.
async fn assert_shell(session: Session, mut rx: Receiver<SessionEvent>) {
    session
        .write(b"echo fernjo$((20+22))\n".to_vec())
        .await
        .unwrap();
    let deadline = Instant::now() + WAIT;
    let mut seen = String::new();
    while !seen.contains("fernjo42") {
        let left = deadline.saturating_duration_since(Instant::now());
        match timeout(left, rx.recv()).await.expect("timeout") {
            Some(SessionEvent::Output(b)) => seen.push_str(&String::from_utf8_lossy(&b)),
            other => panic!("unexpected {other:?}"),
        }
    }
    session.close();
}

#[tokio::test]
async fn ed25519_key_from_credential_logs_in() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(trusting());
    let cred = key_credential(&s.key_ed25519, None);
    let (session, rx) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(cred)),
        Arc::clone(&ia),
    )
    .await
    .unwrap();
    assert_eq!(ia.password_calls() + ia.ki_calls(), 0);
    assert_shell(session, rx).await;
}

#[tokio::test]
async fn rsa_key_from_credential_logs_in() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(trusting());
    // The server only accepts SHA-2 RSA signatures (OpenSSH 8.8+ default), so success proves it.
    let cred = key_credential(&s.key_rsa, None);
    let (session, rx) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(cred)),
        Arc::clone(&ia),
    )
    .await
    .unwrap();
    assert_eq!(ia.password_calls() + ia.ki_calls(), 0);
    assert_shell(session, rx).await;
}

#[tokio::test]
async fn encrypted_key_asks_interaction_for_the_passphrase() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(trusting().passphrase(Some(&s.key_passphrase)));
    let cred = key_credential(&s.key_encrypted, None);
    let (session, rx) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(cred)),
        Arc::clone(&ia),
    )
    .await
    .unwrap();
    assert_eq!(ia.passphrase_calls(), 1);
    assert_eq!(ia.password_calls(), 0);
    assert_shell(session, rx).await;
}

#[tokio::test]
async fn encrypted_key_uses_the_stored_passphrase_without_asking() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(trusting());
    let cred = key_credential(&s.key_encrypted, Some(&s.key_passphrase));
    let (session, rx) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(cred)),
        Arc::clone(&ia),
    )
    .await
    .unwrap();
    assert_eq!(ia.passphrase_calls(), 0);
    assert_shell(session, rx).await;
}

#[tokio::test]
async fn wrong_or_missing_passphrase_counts_as_unreadable() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    // A wrong stored passphrase, and no stored one with the user cancelling the prompt.
    for stored in [Some("not-the-passphrase"), None] {
        let ia = Arc::new(trusting().passphrase(None));
        let cred = key_credential(&s.key_encrypted, stored);
        let Err(e) = Session::connect(
            params(&s, &dir.path().join("kh"), Some(cred)),
            Arc::clone(&ia),
        )
        .await
        else {
            panic!("must not log in")
        };
        assert!(
            matches!(
                e,
                SshError::AuthFailed {
                    key_unreadable: true
                }
            ),
            "{e:?}"
        );
    }
}

#[tokio::test]
async fn unreadable_key_is_skipped_and_reported_in_the_final_error() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    const GARBAGE: &str = "GARBAGE-KEY-TEXT-SENTINEL";
    let garbage = || Credential::PrivateKey {
        key: SecretString::from(GARBAGE),
        passphrase: None,
    };

    // No other method that works: the failure says the key could not be read, and nothing else.
    let ia = Arc::new(trusting());
    let Err(e) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(garbage())),
        Arc::clone(&ia),
    )
    .await
    else {
        panic!("must not log in")
    };
    assert!(
        matches!(
            e,
            SshError::AuthFailed {
                key_unreadable: true
            }
        ),
        "{e:?}"
    );
    assert_eq!(e.code(), "auth_failed");
    assert!(!format!("{e} {e:?}").contains(GARBAGE));

    // A working typed password still succeeds after the key was skipped.
    // (Keyboard-interactive comes before the typed password and is answered wrongly here.)
    let ia = Arc::new(
        trusting()
            .ki_answers(Some(vec!["wrong"]))
            .typed_password(Some(&s.password)),
    );
    let (session, rx) = Session::connect(
        params(&s, &dir.path().join("kh"), Some(garbage())),
        Arc::clone(&ia),
    )
    .await
    .unwrap();
    assert_eq!(ia.password_calls(), 1);
    assert_shell(session, rx).await;
}

#[tokio::test]
async fn keyboard_interactive_round_is_shown_and_answered() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(trusting().ki_answers(Some(vec![&s.password])));
    let (session, rx) = Session::connect(params(&s, &dir.path().join("kh"), None), Arc::clone(&ia))
        .await
        .unwrap();
    assert_eq!(ia.ki_calls(), 1);
    assert_eq!(ia.password_calls(), 0);
    let rounds = ia.ki_rounds();
    assert_eq!(rounds[0].prompts.len(), 1);
    assert!(!rounds[0].prompts[0].echo);
    assert!(!rounds[0].prompts[0].text.is_empty());
    assert_shell(session, rx).await;
}
