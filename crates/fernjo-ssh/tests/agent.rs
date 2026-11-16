//! SSH agent authentication against the OpenSSH test server. Its own test binary on purpose: the
//! other binaries drop `SSH_AUTH_SOCK` so the agent cannot answer for them, this one keeps it.
//! Needs `FERNJO_TEST_SSH_AGENT` (set by `tools/ssh-test-server.sh` where an agent could be
//! started, i.e. not in Git Bash on Windows); skipped without it, an error with `CI` set.

mod support;

use std::sync::Arc;

use fernjo_ssh::{ConnectParams, HostKeyDecision, Session};
use support::ScriptedInteraction;

#[tokio::test]
async fn agent_tries_identities_in_order_and_succeeds_on_the_authorised_one() {
    let Some(s) = support::agent_server() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // The agent holds the unauthorised key first, the authorised Ed25519 key second. No credential,
    // no scripted password or keyboard-interactive answer: only the agent can log in.
    let ia = Arc::new(ScriptedInteraction::new().host_key(HostKeyDecision::TrustOnce));
    let mut p = ConnectParams::new(&s.host, s.port, dir.path().join("kh"));
    p.username = Some(s.user.clone());
    let (session, _rx) = Session::connect(p, Arc::clone(&ia)).await.unwrap();
    assert_eq!(ia.password_calls() + ia.ki_calls(), 0);
    session.close();
}
