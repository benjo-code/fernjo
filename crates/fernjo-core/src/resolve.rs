//! Effective-value resolution: walks from a node up through its folders.

use crate::model::{
    DEFAULT_KEEPALIVE_SECS, DEFAULT_RDP_PORT, DEFAULT_SSH_PORT, Field, Node, NodeKind, NodeType,
    Protocol, ResolvedNode, Settings, Source,
};

/// Resolves the effective value of every setting of `node`.
///
/// `ancestors` runs from the top-level folder down to the direct parent.
pub fn resolve(ancestors: &[&Node], node: &Node) -> ResolvedNode {
    let (kind, protocol, host) = match &node.kind {
        NodeKind::Folder { .. } => (NodeType::Folder, None, None),
        NodeKind::Connection { protocol, host } => {
            (NodeType::Connection, Some(*protocol), Some(host.clone()))
        }
    };
    let default_port = protocol.map(|p| match p {
        Protocol::Ssh => DEFAULT_SSH_PORT,
        Protocol::Rdp => DEFAULT_RDP_PORT,
    });
    ResolvedNode {
        id: node.id,
        kind,
        name: node.name.clone(),
        description: node.description.clone(),
        protocol,
        host,
        username: field(ancestors, node, |s| s.username.clone(), None),
        port: field(ancestors, node, |s| s.port, default_port),
        keepalive_secs: field(
            ancestors,
            node,
            |s| s.keepalive_secs,
            Some(DEFAULT_KEEPALIVE_SECS),
        ),
        credential: field(ancestors, node, |s| s.credential, None),
    }
}

fn field<T: Clone>(
    ancestors: &[&Node],
    node: &Node,
    get: impl Fn(&Settings) -> Option<T>,
    default: Option<T>,
) -> Field<T> {
    let local = get(&node.settings);
    if local.is_some() {
        return Field {
            effective: local.clone(),
            local,
            source: Source::Local,
        };
    }
    for folder in ancestors.iter().rev() {
        if let Some(value) = get(&folder.settings) {
            return Field {
                local: None,
                effective: Some(value),
                source: Source::Folder {
                    id: folder.id,
                    name: folder.name.clone(),
                },
            };
        }
    }
    Field {
        local: None,
        effective: default,
        source: Source::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn folder(name: &str, settings: Settings, children: Vec<Node>) -> Node {
        Node {
            id: NodeId::new_v4(),
            name: name.to_string(),
            description: String::new(),
            settings,
            kind: NodeKind::Folder { children },
        }
    }

    fn conn(name: &str, protocol: Protocol, settings: Settings) -> Node {
        Node {
            id: NodeId::new_v4(),
            name: name.to_string(),
            description: String::new(),
            settings,
            kind: NodeKind::Connection {
                protocol,
                host: "h.example".to_string(),
            },
        }
    }

    fn port(p: u16) -> Settings {
        Settings {
            port: Some(p),
            ..Settings::default()
        }
    }

    #[test]
    fn ssh_port_defaults_to_22_and_rdp_to_3389() {
        let ssh = resolve(&[], &conn("a", Protocol::Ssh, Settings::default()));
        assert_eq!(ssh.port.effective, Some(22));
        assert_eq!(ssh.port.source, Source::Default);
        let rdp = resolve(&[], &conn("b", Protocol::Rdp, Settings::default()));
        assert_eq!(rdp.port.effective, Some(3389));
        assert_eq!(rdp.port.source, Source::Default);
    }

    #[test]
    fn folder_port_has_no_default() {
        let f = resolve(&[], &folder("f", Settings::default(), vec![]));
        assert_eq!(f.port.effective, None);
        assert_eq!(f.port.source, Source::Default);
    }

    #[test]
    fn keepalive_defaults_to_30() {
        let r = resolve(&[], &conn("a", Protocol::Ssh, Settings::default()));
        assert_eq!(r.keepalive_secs.effective, Some(30));
        assert_eq!(r.keepalive_secs.source, Source::Default);
    }

    #[test]
    fn username_has_no_default() {
        let r = resolve(&[], &conn("a", Protocol::Ssh, Settings::default()));
        assert_eq!(r.username.effective, None);
        assert_eq!(r.username.source, Source::Default);
    }

    #[test]
    fn local_value_wins_even_if_equal_to_inherited() {
        let f = folder("F", port(2200), vec![]);
        let c = conn("c", Protocol::Ssh, port(2200));
        let r = resolve(&[&f], &c);
        assert_eq!(r.port.effective, Some(2200));
        assert_eq!(r.port.local, Some(2200));
        assert_eq!(r.port.source, Source::Local);
    }

    #[test]
    fn nearest_folder_wins_over_three_levels() {
        let a = folder("A", port(2200), vec![]);
        let b = folder("B", port(2300), vec![]);
        let c = conn("c", Protocol::Ssh, Settings::default());
        let r = resolve(&[&a, &b], &c);
        assert_eq!(r.port.effective, Some(2300));
        assert_eq!(r.port.local, None);
        assert_eq!(
            r.port.source,
            Source::Folder {
                id: b.id,
                name: "B".to_string()
            }
        );
    }

    #[test]
    fn inherits_username_from_grandparent() {
        let a = folder(
            "A",
            Settings {
                username: Some("ops".to_string()),
                ..Settings::default()
            },
            vec![],
        );
        let b = folder("B", Settings::default(), vec![]);
        let c = conn("c", Protocol::Ssh, Settings::default());
        let r = resolve(&[&a, &b], &c);
        assert_eq!(r.username.effective.as_deref(), Some("ops"));
        assert_eq!(
            r.username.source,
            Source::Folder {
                id: a.id,
                name: "A".to_string()
            }
        );
    }

    fn cred(id: CredentialId) -> Settings {
        Settings {
            credential: Some(id),
            ..Settings::default()
        }
    }

    #[test]
    fn credential_inherits_from_grandparent_with_source() {
        let id = CredentialId::new_v4();
        let a = folder("A", cred(id), vec![]);
        let b = folder("B", Settings::default(), vec![]);
        let c = conn("c", Protocol::Ssh, Settings::default());
        let r = resolve(&[&a, &b], &c);
        assert_eq!(r.credential.effective, Some(id));
        assert_eq!(r.credential.local, None);
        assert_eq!(
            r.credential.source,
            Source::Folder {
                id: a.id,
                name: "A".to_string()
            }
        );
    }

    #[test]
    fn local_credential_wins_over_folder() {
        let (folder_id, local_id) = (CredentialId::new_v4(), CredentialId::new_v4());
        let f = folder("F", cred(folder_id), vec![]);
        let c = conn("c", Protocol::Ssh, cred(local_id));
        let r = resolve(&[&f], &c);
        assert_eq!(r.credential.effective, Some(local_id));
        assert_eq!(r.credential.local, Some(local_id));
        assert_eq!(r.credential.source, Source::Local);
    }

    #[test]
    fn credential_has_no_default() {
        let r = resolve(&[], &conn("a", Protocol::Ssh, Settings::default()));
        assert_eq!(r.credential.effective, None);
        assert_eq!(r.credential.source, Source::Default);
    }

    #[test]
    fn json_shape() {
        let c = conn("web-01", Protocol::Ssh, Settings::default());
        let v = serde_json::to_value(&c).unwrap();
        assert_eq!(v["type"], "connection");
        assert_eq!(v["protocol"], "ssh");
        assert_eq!(v["host"], "h.example");
        assert_eq!(v["settings"], serde_json::json!({}));
        let back: Node = serde_json::from_value(v).unwrap();
        assert_eq!(back, c);

        let f = folder("F", port(2200), vec![c]);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(v["type"], "folder");
        assert_eq!(v["settings"], serde_json::json!({ "port": 2200 }));
        assert_eq!(v["children"][0]["type"], "connection");
        let back: Node = serde_json::from_value(v).unwrap();
        assert_eq!(back, f);
    }
}
