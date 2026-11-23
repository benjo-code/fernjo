//! Quick connect: parses `[ssh://|rdp://]user@host[:port]` into a [`QuickTarget`].
//!
//! Syntax only. Whether the host is acceptable for `known_hosts` and certificate pins is decided
//! by `fernjo_ssh::validate_host` where the target is resolved (the webview is not trusted).

use serde::{Deserialize, Serialize};

use crate::{DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, Protocol};

/// A connection typed into the quick connect box. It is never stored in the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickTarget {
    pub protocol: Protocol,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
}

/// Why the input is not a target; `code` is stable, see [`QuickError::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickError {
    pub code: &'static str,
}

impl QuickError {
    /// A plain sentence for the UI.
    pub fn message(self) -> &'static str {
        match self.code {
            "quick_empty" => "type a target like user@host",
            "quick_user_required" => "an SSH target needs a user name (user@host)",
            "quick_bad_scheme" => "the scheme must be ssh:// or rdp://",
            "quick_bad_port" => "the port must be a number from 1 to 65535",
            _ => "this is not a valid target (use [ssh://|rdp://]user@host[:port])",
        }
    }
}

const fn err(code: &'static str) -> QuickError {
    QuickError { code }
}

/// Parses `[ssh://|rdp://]user@host[:port]`. Without a scheme the protocol is SSH. An RDP user is
/// optional (it can be typed at the prompt). An IPv6 host is written `[::1]`.
///
/// # Errors
/// `quick_empty`, `quick_user_required`, `quick_bad_scheme`, `quick_bad_port`, `quick_bad_target`.
pub fn parse_quick(input: &str) -> Result<QuickTarget, QuickError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(err("quick_empty"));
    }
    let (protocol, rest) = match input.split_once("://") {
        Some((scheme, rest)) => {
            let protocol = match scheme.to_ascii_lowercase().as_str() {
                "ssh" => Protocol::Ssh,
                "rdp" => Protocol::Rdp,
                _ => return Err(err("quick_bad_scheme")),
            };
            (protocol, rest)
        }
        None => (Protocol::Ssh, input),
    };
    if rest.matches('@').count() > 1 {
        return Err(err("quick_bad_target"));
    }
    let (user, host_part) = match rest.split_once('@') {
        Some((user, host_part)) => {
            if user.is_empty() || user.chars().any(char::is_whitespace) {
                return Err(err("quick_bad_target"));
            }
            (Some(user.to_string()), host_part)
        }
        None => (None, rest),
    };
    if user.is_none() && protocol == Protocol::Ssh {
        // report a missing user only for something that otherwise looks like a host
        if host_part.is_empty() {
            return Err(err("quick_bad_target"));
        }
        return Err(err("quick_user_required"));
    }
    let (host, port_text) = split_host_port(host_part)?;
    if host.is_empty()
        || host
            .chars()
            .any(|c| c.is_whitespace() || "/\\;@[]".contains(c))
    {
        return Err(err("quick_bad_target"));
    }
    let port = match port_text {
        None => match protocol {
            Protocol::Ssh => DEFAULT_SSH_PORT,
            Protocol::Rdp => DEFAULT_RDP_PORT,
        },
        Some(text) => match text.parse::<u32>() {
            Ok(p) if (1..=65535).contains(&p) => p as u16,
            _ => return Err(err("quick_bad_port")),
        },
    };
    Ok(QuickTarget {
        protocol,
        host: host.to_string(),
        port,
        user,
    })
}

/// Splits `host`, `host:port`, `[v6]` and `[v6]:port`.
fn split_host_port(part: &str) -> Result<(&str, Option<&str>), QuickError> {
    if let Some(inner) = part.strip_prefix('[') {
        let (host, after) = inner.split_once(']').ok_or(err("quick_bad_target"))?;
        return match after {
            "" => Ok((host, None)),
            _ => match after.strip_prefix(':') {
                Some(port) => Ok((host, Some(port))),
                None => Err(err("quick_bad_target")),
            },
        };
    }
    match part.split_once(':') {
        None => Ok((part, None)),
        // a second colon means a bare IPv6 address, which must be bracketed
        Some((_, port)) if port.contains(':') => Err(err("quick_bad_target")),
        Some((host, port)) => Ok((host, Some(port))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(input: &str) -> QuickTarget {
        parse_quick(input).unwrap()
    }

    fn code(input: &str) -> &'static str {
        parse_quick(input).unwrap_err().code
    }

    #[test]
    fn parses_user_host() {
        assert_eq!(
            ok("root@example.com"),
            QuickTarget {
                protocol: Protocol::Ssh,
                host: "example.com".into(),
                port: 22,
                user: Some("root".into()),
            }
        );
    }

    #[test]
    fn parses_scheme_and_port() {
        let t = ok("rdp://admin@10.0.0.5:3390");
        assert_eq!(t.protocol, Protocol::Rdp);
        assert_eq!((t.host.as_str(), t.port), ("10.0.0.5", 3390));
        assert_eq!(t.user.as_deref(), Some("admin"));
        assert_eq!(ok("ssh://u@h:2222").port, 2222);
    }

    #[test]
    fn rdp_user_is_optional() {
        let t = ok("rdp://server");
        assert_eq!((t.user, t.port), (None, 3389));
    }

    #[test]
    fn ssh_without_user_is_refused() {
        assert_eq!(code("example.com"), "quick_user_required");
        assert_eq!(code("ssh://example.com:22"), "quick_user_required");
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(ok("  u@h \n"), ok("u@h"));
        assert_eq!(code("   "), "quick_empty");
        assert_eq!(code(""), "quick_empty");
    }

    #[test]
    fn parses_bracketed_ipv6_with_port() {
        let t = ok("u@[::1]:2200");
        assert_eq!((t.host.as_str(), t.port), ("::1", 2200));
        assert_eq!(ok("u@[fe80::1%eth0]").host, "fe80::1%eth0");
        assert_eq!(code("u@::1"), "quick_bad_target");
        assert_eq!(code("u@[::1"), "quick_bad_target");
        assert_eq!(code("u@[::1]x"), "quick_bad_target");
    }

    #[test]
    fn rejects_double_at_and_empty_parts() {
        assert_eq!(code("a@b@c"), "quick_bad_target");
        assert_eq!(code("@host"), "quick_bad_target");
        assert_eq!(code("u@"), "quick_bad_target");
        assert_eq!(code("u a@host"), "quick_bad_target");
        assert_eq!(code("rdp://"), "quick_bad_target");
    }

    #[test]
    fn rejects_port_zero_and_65536() {
        assert_eq!(code("u@h:0"), "quick_bad_port");
        assert_eq!(code("u@h:65536"), "quick_bad_port");
        assert_eq!(code("u@h:"), "quick_bad_port");
        assert_eq!(code("u@h:ssh"), "quick_bad_port");
        assert_eq!(ok("u@h:65535").port, 65535);
    }

    #[test]
    fn rejects_unknown_scheme() {
        assert_eq!(code("http://u@h"), "quick_bad_scheme");
        assert_eq!(code("vnc://h"), "quick_bad_scheme");
    }

    #[test]
    fn rejects_host_with_slash_semicolon_or_space() {
        assert_eq!(code("u@a/b"), "quick_bad_target");
        assert_eq!(code("u@a;b"), "quick_bad_target");
        assert_eq!(code("u@bad host"), "quick_bad_target");
    }

    #[test]
    fn serialises_camel_case() {
        let json = serde_json::to_value(ok("rdp://u@h")).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"protocol": "rdp", "host": "h", "port": 3389, "user": "u"})
        );
    }
}
