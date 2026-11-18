//! Connection parameters and credentials. Secrets never appear in `Debug` output.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;

use crate::known_hosts::KnownHosts;

/// Credential to authenticate with. `Debug` is redacted.
pub enum Credential {
    Password(SecretString),
    PrivateKey {
        key: SecretString,
        passphrase: Option<SecretString>,
    },
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password(_) => f.write_str("Credential::Password(<redacted>)"),
            Self::PrivateKey { passphrase, .. } => f
                .debug_struct("Credential::PrivateKey")
                .field("key", &"<redacted>")
                .field("passphrase", &passphrase.as_ref().map(|_| "<redacted>"))
                .finish(),
        }
    }
}

/// Everything needed to open one SSH connection.
#[derive(Debug)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub keepalive_secs: u32,
    pub credential: Option<Credential>,
    pub known_hosts: PathBuf,
    /// Hosts this device has confirmed itself (an OpenSSH-format file outside the synced data
    /// folder). A key found in `known_hosts` is trusted silently only when this file has it too;
    /// otherwise the user is asked once. `None` and no `confirmed_in_memory`: the layer is off.
    pub confirmed_hosts: Option<PathBuf>,
    /// Process-lifetime stand-in for `confirmed_hosts` when the device has no folder for it
    /// (a host then asks once per run). Used only when `confirmed_hosts` is `None`.
    pub confirmed_in_memory: Option<KnownHosts>,
    pub cols: u32,
    pub rows: u32,
    pub connect_timeout: Duration,
}

impl ConnectParams {
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

    /// Parameters with defaults: no username or credential, keepalive 30 s, 80x24, 15 s timeout.
    pub fn new(host: impl Into<String>, port: u16, known_hosts: PathBuf) -> Self {
        Self {
            host: host.into(),
            port,
            username: None,
            keepalive_secs: 30,
            credential: None,
            known_hosts,
            confirmed_hosts: None,
            confirmed_in_memory: None,
            cols: 80,
            rows: 24,
            connect_timeout: Self::DEFAULT_TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_contains_a_secret() {
        const S: &str = "SENTINEL-SECRET";
        let creds = [
            Credential::Password(SecretString::from(S)),
            Credential::PrivateKey {
                key: SecretString::from(S),
                passphrase: Some(SecretString::from(S)),
            },
        ];
        for c in creds {
            assert!(!format!("{c:?}").contains(S));
            assert!(!format!("{c:#?}").contains(S));
            let mut p = ConnectParams::new("h", 22, PathBuf::from("kh"));
            p.credential = Some(c);
            assert!(!format!("{p:?}").contains(S));
            assert!(!format!("{p:#?}").contains(S));
        }
    }

    #[test]
    fn new_fills_defaults() {
        let p = ConnectParams::new("h", 2222, PathBuf::from("kh"));
        assert_eq!(p.connect_timeout, ConnectParams::DEFAULT_TIMEOUT);
        assert_eq!((p.cols, p.rows, p.keepalive_secs), (80, 24, 30));
        assert!(p.username.is_none() && p.credential.is_none());
    }
}
