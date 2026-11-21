//! What a connection needs to start.

use std::fmt;
use std::time::Duration;

use secrecy::SecretString;

use crate::CertStore;

/// Everything `Session::connect` needs. The password is a [`SecretString`] and the `Debug` output
/// shows neither it nor the user name.
#[derive(Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    /// An empty user name makes the session ask [`crate::Interaction::username`].
    pub username: String,
    /// An empty password makes the session ask [`crate::Interaction::password`] (after the
    /// certificate decision, so the password is never typed for a server that was not trusted).
    pub password: SecretString,
    pub domain: Option<String>,
    /// Desktop size to ask for at connect time.
    pub width: u16,
    pub height: u16,
    /// Bits per pixel: 16, 24 or 32.
    pub color_depth: u8,
    /// Share the text clipboard.
    pub clipboard: bool,
    /// Offer the display control channel so the desktop can follow the tab size later.
    pub dynamic_resize: bool,
    pub certs: CertStore,
    /// Bounds the network steps of connecting (not the time a person needs to answer a prompt).
    pub connect_timeout: Duration,
}

impl ConnectParams {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        password: SecretString,
    ) -> ConnectParams {
        ConnectParams {
            host: host.into(),
            port,
            username: username.into(),
            password,
            domain: None,
            width: 1280,
            height: 800,
            color_depth: 32,
            clipboard: true,
            dynamic_resize: true,
            certs: CertStore::in_memory(),
            connect_timeout: Duration::from_secs(15),
        }
    }
}

impl fmt::Debug for ConnectParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectParams")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &"<redacted>")
            .field("password", &"<redacted>")
            .field("domain", &self.domain.as_ref().map(|_| "<redacted>"))
            .field("width", &self.width)
            .field("height", &self.height)
            .field("color_depth", &self.color_depth)
            .field("clipboard", &self.clipboard)
            .field("dynamic_resize", &self.dynamic_resize)
            .field("connect_timeout", &self.connect_timeout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;

    #[test]
    fn defaults_match_the_documented_ones() {
        let p = ConnectParams::new("dc01", 3389, "admin", SecretString::from("pw".to_string()));
        assert_eq!((p.width, p.height, p.color_depth), (1280, 800, 32));
        assert!(p.clipboard && p.dynamic_resize);
        assert_eq!(p.connect_timeout, Duration::from_secs(15));
        assert_eq!(p.domain, None);
        assert_eq!(p.password.expose_secret(), "pw");
    }

    #[test]
    fn debug_shows_no_secret_user_or_domain() {
        let mut p = ConnectParams::new(
            "dc01",
            3389,
            "sentinel-user",
            SecretString::from("sentinel-password".to_string()),
        );
        p.domain = Some("sentinel-domain".to_string());
        let text = format!("{p:?}");
        for secret in ["sentinel-user", "sentinel-password", "sentinel-domain"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(text.contains("dc01"));
    }
}
