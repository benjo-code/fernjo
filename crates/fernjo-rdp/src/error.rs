//! Errors of the RDP layer. None of them ever carries a password or a username.

/// Why an RDP operation failed. Every variant has a stable machine-readable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RdpError {
    #[error("the host name is not valid")]
    InvalidHost,
    #[error("could not connect to the host")]
    ConnectFailed,
    #[error("the connection timed out")]
    Timeout,
    #[error("the server refused the login")]
    AuthFailed,
    #[error("the secure connection to the server failed")]
    TlsFailed,
    #[error(
        "the server does not offer Network Level Authentication (NLA); fernjo only connects to RDP servers with NLA, so the password is only ever sent inside NLA (CredSSP)"
    )]
    NlaRequired,
    #[error("the server certificate is not supported")]
    CertificateUnsupported,
    #[error("the server certificate was not accepted")]
    CertificateRejected,
    #[error("the remote desktop has a size that is not supported")]
    DesktopTooLarge,
    #[error("the server broke the RDP protocol")]
    Protocol,
    #[error("the connection to the server was lost")]
    Lost,
    #[error("a file could not be read or written")]
    Io,
}

/// A server announced a virtual channel message above the session's cap; the session ends with
/// `CloseReason::ChannelTooLarge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the server announced a channel message that is too large")]
pub(crate) struct ChannelTooLarge;

/// A server sent a display update beyond the client's limits (a fragmented fast-path update above
/// its cap, or a pointer outside the advertised cache or size); the session ends with
/// `CloseReason::UpdateOutOfLimits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the server sent a display update beyond the client's limits")]
pub(crate) struct UpdateOutOfLimits;

impl RdpError {
    /// Stable code shown to the UI and used in tests.
    pub fn code(&self) -> &'static str {
        match self {
            RdpError::InvalidHost => "invalid_host",
            RdpError::ConnectFailed => "connect_failed",
            RdpError::Timeout => "timeout",
            RdpError::AuthFailed => "auth_failed",
            RdpError::TlsFailed => "tls_failed",
            RdpError::CertificateUnsupported => "certificate_unsupported",
            RdpError::NlaRequired => "nla_required",
            RdpError::CertificateRejected => "certificate_rejected",
            RdpError::DesktopTooLarge => "desktop_too_large",
            RdpError::Protocol => "protocol",
            RdpError::Lost => "lost",
            RdpError::Io => "io",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable() {
        let all = [
            (RdpError::InvalidHost, "invalid_host"),
            (RdpError::ConnectFailed, "connect_failed"),
            (RdpError::Timeout, "timeout"),
            (RdpError::AuthFailed, "auth_failed"),
            (RdpError::TlsFailed, "tls_failed"),
            (RdpError::CertificateUnsupported, "certificate_unsupported"),
            (RdpError::NlaRequired, "nla_required"),
            (RdpError::CertificateRejected, "certificate_rejected"),
            (RdpError::DesktopTooLarge, "desktop_too_large"),
            (RdpError::Protocol, "protocol"),
            (RdpError::Lost, "lost"),
            (RdpError::Io, "io"),
        ];
        for (error, code) in all {
            assert_eq!(error.code(), code);
        }
    }
}
