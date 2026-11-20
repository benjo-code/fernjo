//! A connection killed on the server side ends as `ConnectionLost`. Its own test binary because
//! it kills every sshd connection process of the shared test server, which would drop the
//! connections of the tests in `connect.rs` that run in parallel. Skips without
//! `FERNJO_TEST_SSH_ADDR` (fails with `CI` set), see `support`.

mod support;

use std::sync::Arc;
use std::time::Duration;

use fernjo_ssh::{CloseReason, ConnectParams, Credential, HostKeyDecision, Session, SessionEvent};
use secrecy::SecretString;
use support::{ScriptedInteraction, TestServer};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(10);

fn params(s: &TestServer, known_hosts: &std::path::Path) -> ConnectParams {
    let mut p = ConnectParams::new(&s.host, s.port, known_hosts.to_path_buf());
    p.username = Some(s.user.clone());
    p.credential = Some(Credential::Password(SecretString::from(s.password.clone())));
    p
}

/// Drops the connection from the server side. `tools/ssh-test-server.sh` has no way to drop one
/// connection, so the per-connection sshd processes are killed:
/// `docker exec fernjo-ssh-test pkill -9 -f sshd-session` (the image's sshd runs
/// each connection in an `sshd-session.pam: ...` process; the listener is `sshd.pam: ...` and
/// keeps running for the other tests).
#[tokio::test]
async fn a_killed_connection_is_connection_lost() {
    let Some(s) = support::server() else { return };
    let dir = tempfile::tempdir().unwrap();
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let (session, mut rx) = Session::connect(params(&s, &dir.path().join("kh")), ia)
        .await
        .unwrap();
    session
        .write(
            b"echo up$((1+1))
"
            .to_vec(),
        )
        .await
        .unwrap();
    timeout(WAIT, async {
        let mut seen = String::new();
        while !seen.contains("up2") {
            match rx.recv().await {
                Some(SessionEvent::Output(b)) => seen.push_str(&String::from_utf8_lossy(&b)),
                Some(SessionEvent::Closed(r)) => panic!("closed ({r:?}) before the shell was up"),
                None => panic!("events channel ended before the shell was up"),
            }
        }
    })
    .await
    .expect("timeout waiting for the shell");
    let killed = std::process::Command::new("docker")
        .args([
            "exec",
            "fernjo-ssh-test",
            "pkill",
            "-9",
            "-f",
            "sshd-session",
        ])
        .status()
        .expect("docker is needed for this test");
    assert!(killed.success(), "pkill found no connection to kill");
    let end = timeout(WAIT, async {
        loop {
            match rx.recv().await {
                Some(SessionEvent::Closed(r)) => return r,
                Some(_) => {}
                None => panic!("events channel ended without a Closed event"),
            }
        }
    })
    .await
    .expect("timeout waiting for the session to end");
    assert_eq!(end, CloseReason::ConnectionLost);
}
