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

impl RdpError {
    /// Stable code shown to the UI and used in tests.
    pub fn code(&self) -> &'static str {
        match self {
            RdpError::InvalidHost => "invalid_host",
            RdpError::ConnectFailed => "connect_failed",
            RdpError::Timeout => "timeout",
            RdpError::AuthFailed => "auth_failed",
            RdpError::TlsFailed => "tls_failed",
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
