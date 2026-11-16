//! Errors of the SSH layer. No variant stores user-supplied secret text.

/// Failure of an SSH operation. `Display` output is a fixed sentence per variant.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("Could not connect to the host.")]
    ConnectFailed,
    #[error("The connection timed out.")]
    Timeout,
    #[error("The host key was rejected.")]
    HostKeyRejected,
    #[error("The host key has changed and was rejected.")]
    HostKeyChangedRejected,
    #[error("Authentication failed.")]
    AuthFailed { key_unreadable: bool },
    #[error("The connection was lost.")]
    ConnectionLost,
    #[error("The session is closed.")]
    Closed,
    #[error("SSH protocol error.")]
    Protocol,
    #[error("I/O error.")]
    Io,
}

impl SshError {
    /// Stable machine-readable code for the UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConnectFailed => "connect_failed",
            Self::Timeout => "timeout",
            Self::HostKeyRejected => "host_key_rejected",
            Self::HostKeyChangedRejected => "host_key_changed_rejected",
            Self::AuthFailed { .. } => "auth_failed",
            Self::ConnectionLost => "connection_lost",
            Self::Closed => "closed",
            Self::Protocol => "protocol",
            Self::Io => "io",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_display_is_secret_free() {
        // No variant takes free text, so SENTINEL has no way in; the fixed sentences are the guarantee.
        const SENTINEL: &str = "SENTINEL";
        let all = [
            SshError::ConnectFailed,
            SshError::Timeout,
            SshError::HostKeyRejected,
            SshError::HostKeyChangedRejected,
            SshError::AuthFailed {
                key_unreadable: true,
            },
            SshError::ConnectionLost,
            SshError::Closed,
            SshError::Protocol,
            SshError::Io,
        ];
        let codes: Vec<&str> = all.iter().map(SshError::code).collect();
        assert_eq!(
            codes,
            [
                "connect_failed",
                "timeout",
                "host_key_rejected",
                "host_key_changed_rejected",
                "auth_failed",
                "connection_lost",
                "closed",
                "protocol",
                "io"
            ]
        );
        for e in &all {
            let s = e.to_string();
            assert!(!s.is_empty());
            assert!(!s.contains(SENTINEL));
            assert!(!format!("{e:?}").contains(SENTINEL));
        }
    }
}
