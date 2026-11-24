//! Caps and clean-up for text that comes from a file the user picked (a fernjo export or a
//! backup). The numbers are the mRemoteNG importer's (`mremoteng::file::MAX_*_CHARS`), so
//! every import path bounds names, descriptions and hosts the same way.

use fernjo_core::{Node, NodeKind};

use crate::mremoteng::file::{MAX_DESCRIPTION_CHARS, MAX_HOST_CHARS, MAX_NAME_CHARS};

/// Longest attacker-chosen string that may appear in an error message or a report (characters).
pub const MAX_MESSAGE_CHARS: usize = 256;

/// The first `max` characters of `s` (a cut on a character boundary, never inside one).
pub fn cut_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_owned(),
        Some((i, _)) => s[..i].to_owned(),
    }
}

/// `s` without control characters (`keep_newlines` keeps `\n`, `\r` and `\t`), cut to `max`.
pub fn clean(s: &str, max: usize, keep_newlines: bool) -> String {
    let kept: String = s
        .chars()
        .filter(|c| !c.is_control() || (keep_newlines && matches!(c, '\n' | '\r' | '\t')))
        .take(max + 1)
        .collect();
    cut_chars(&kept, max)
}

/// What `sanitize_nodes` changed, for one aggregate report line.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cleaned {
    /// Names, user names, descriptions and hosts that were shortened or had control characters removed.
    pub changed: usize,
    /// Hosts longer than the cap, imported with an empty host.
    pub hosts_dropped: usize,
}

/// Applies the caps to the names, user names, descriptions and hosts of `nodes` and everything below
/// them. Iterative (no recursion on attacker-chosen depth).
pub fn sanitize_nodes(nodes: &mut [Node]) -> Cleaned {
    let mut out = Cleaned::default();
    let mut stack: Vec<&mut [Node]> = vec![nodes];
    while let Some(level) = stack.pop() {
        for node in level {
            let name = clean(&node.name, MAX_NAME_CHARS, false);
            let name = if name.is_empty() && !node.name.is_empty() {
                "unnamed".to_owned()
            } else {
                name
            };
            if name != node.name {
                node.name = name;
                out.changed += 1;
            }
            if let Some(user) = &node.settings.username {
                let cleaned = clean(user, MAX_NAME_CHARS, false);
                let cleaned = (!cleaned.is_empty()).then_some(cleaned);
                if cleaned.as_deref() != Some(user.as_str()) {
                    node.settings.username = cleaned;
                    out.changed += 1;
                }
            }
            let description = clean(&node.description, MAX_DESCRIPTION_CHARS, true);
            if description != node.description {
                node.description = description;
                out.changed += 1;
            }
            match &mut node.kind {
                NodeKind::Folder { children } => stack.push(children),
                NodeKind::Connection { host, .. } => {
                    if host.chars().nth(MAX_HOST_CHARS).is_some() {
                        host.clear();
                        out.hosts_dropped += 1;
                        out.changed += 1;
                    } else {
                        let h = clean(host, MAX_HOST_CHARS, false);
                        if h != *host {
                            *host = h;
                            out.changed += 1;
                        }
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_node(name: &str, user: &str) -> Node {
        Node {
            id: fernjo_core::NodeId::new_v4(),
            name: name.to_owned(),
            description: String::new(),
            settings: fernjo_core::Settings {
                username: Some(user.to_owned()),
                ..Default::default()
            },
            kind: NodeKind::Connection {
                protocol: fernjo_core::Protocol::Ssh,
                host: "h".to_owned(),
            },
        }
    }

    #[test]
    fn sanitize_nodes_cleans_and_caps_the_username() {
        let long = format!("root{}", "x".repeat(400));
        let mut nodes = vec![
            user_node("a", &long),
            user_node("b", ""),
            user_node("c", "alice"),
        ];
        let out = sanitize_nodes(&mut nodes);
        let first = nodes[0].settings.username.clone().unwrap();
        assert!(first.starts_with("rootx") && !first.contains(''));
        assert_eq!(first.chars().count(), MAX_NAME_CHARS);
        assert_eq!(nodes[1].settings.username, None);
        assert_eq!(nodes[2].settings.username.as_deref(), Some("alice"));
        assert_eq!(out.changed, 2);
    }

    #[test]
    fn cut_chars_never_splits_a_character() {
        assert_eq!(cut_chars("aäb", 2), "aä");
        assert_eq!(cut_chars("abc", 5), "abc");
        assert_eq!(cut_chars(&"é".repeat(300), 256).chars().count(), 256);
    }

    #[test]
    fn clean_strips_controls_and_keeps_newlines_when_asked() {
        assert_eq!(clean("a\u{1b}[2Jb\u{0}", 10, false), "a[2Jb");
        assert_eq!(clean("a\nb\u{7}", 10, true), "a\nb");
        assert_eq!(clean("a\nb", 10, false), "ab");
    }
}
