//! Access to the user's SSH agent (Unix socket from `SSH_AUTH_SOCK`, the OpenSSH named pipe on
//! Windows) and the order in which its identities are tried.

use russh::keys::PublicKey;
use russh::keys::agent::client::{AgentClient, AgentStream};

/// Agent connection with the transport type erased, so every platform shares one type.
pub(crate) type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin>>;

/// Connects to the agent. Any failure (no agent, no socket, refused) is `None`: the agent is an
/// optional extra, never a reason to fail. The caller bounds the wait (a busy pipe is retried).
#[cfg(unix)]
pub(crate) async fn connect() -> Option<Agent> {
    AgentClient::connect_env()
        .await
        .ok()
        .map(AgentClient::dynamic)
}

#[cfg(windows)]
pub(crate) async fn connect() -> Option<Agent> {
    AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent")
        .await
        .ok()
        .map(AgentClient::dynamic)
}

#[cfg(not(any(unix, windows)))]
pub(crate) async fn connect() -> Option<Agent> {
    None
}

/// The identity equal to `preferred` first, the others in their original order. Keys are compared
/// by key data, so differing comments do not matter.
pub(crate) fn order_identities(
    identities: Vec<PublicKey>,
    preferred: Option<&PublicKey>,
) -> Vec<PublicKey> {
    let Some(preferred) = preferred else {
        return identities;
    };
    let (mut first, rest): (Vec<_>, Vec<_>) = identities
        .into_iter()
        .partition(|k| k.key_data() == preferred.key_data());
    first.extend(rest);
    first
}

#[cfg(test)]
mod tests {
    use super::*;
    // Public keys only (generated once with ssh-keygen; no private part exists here).
    const KEYS: [&str; 4] = [
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOrjLK0oCIUKN+AJZ0h187ued77g5PDbmhVRKMtIlW5L a",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEKzL74bw0hNw5Xa46bZEQj4eOQkui/Am8RyO6YUOeKe b",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHE4kU/Bgsk/qxefmlOxbwLQCFF986d/tmRJ8UzLaBgS c",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFVWIYsVxLB2X7RV+P7DDiY1TXGmiNbwZn9GsJTUtRHt x",
    ];

    fn key(i: usize) -> PublicKey {
        PublicKey::from_openssh(KEYS[i]).unwrap()
    }

    #[test]
    fn preferred_identity_moves_to_the_front_and_the_rest_keep_order() {
        let (a, b, c) = (key(0), key(1), key(2));
        let mut preferred = c.clone();
        preferred.set_comment("other comment");
        let ordered = order_identities(vec![a.clone(), b.clone(), c.clone()], Some(&preferred));
        assert_eq!(ordered, [c.clone(), a.clone(), b.clone()]);
        // No preference, or one the agent does not hold: unchanged.
        assert_eq!(
            order_identities(vec![a.clone(), b.clone()], None),
            [a.clone(), b.clone()]
        );
        assert_eq!(
            order_identities(vec![a.clone(), b.clone()], Some(&key(3))),
            [a, b]
        );
    }
}
