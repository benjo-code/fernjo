use super::*;
use crate::commands;
use crate::session_commands::{
    answer_prompt, close_session, open_session, reconnect_session, resume_session, session_ack,
    session_input, session_resize,
};
use crate::vault_commands::{create_credential, vault_create, vault_lock, vault_unlock};
use fernjo_core::Protocol;
use secrecy::SecretString;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(15);
const MASTER: &str = "SENTINEL-master-1a2b3c";

fn code<T>(r: Result<T, CommandError>) -> String {
    match r {
        Ok(_) => "ok".to_string(),
        Err(e) => e.code,
    }
}

// ---------- no server needed ----------

#[test]
fn ki_prompts_are_capped_in_count_and_length() {
    use fernjo_ssh::KiPrompt;
    let long = |n: usize| "\u{e9}".repeat(n);
    let round = KiRound {
        name: long(5000),
        instructions: long(5000),
        prompts: (0..40)
            .map(|i| KiPrompt {
                text: if i == 0 { long(5000) } else { format!("p{i}") },
                echo: i % 2 == 0,
            })
            .collect(),
    };
    let PromptView::KeyboardInteractive {
        name,
        instructions,
        prompts,
    } = cap_ki_round(round)
    else {
        panic!("wrong view");
    };
    assert_eq!(prompts.len(), 16);
    assert_eq!(
        prompts[0].text.chars().count(),
        1025,
        "1024 plus the ellipsis"
    );
    assert!(prompts[0].text.ends_with('\u{2026}'));
    assert_eq!(prompts[1].text, "p1");
    assert!(prompts[2].echo && !prompts[1].echo);
    assert_eq!(instructions.chars().count(), 4097);
    assert_eq!(name.chars().count(), 1025);
    // a round within the limits passes unchanged
    let small = KiRound {
        name: "n".into(),
        instructions: "i".into(),
        prompts: vec![KiPrompt {
            text: "Password: ".into(),
            echo: false,
        }],
    };
    assert_eq!(
        cap_ki_round(small),
        PromptView::KeyboardInteractive {
            name: "n".into(),
            instructions: "i".into(),
            prompts: vec![KiPromptView {
                text: "Password: ".into(),
                echo: false
            }]
        }
    );
}

fn answer(v: Value) -> PromptAnswer {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn prompt_hub_delivers_the_answer_to_the_waiting_ask() {
    let hub = PromptHub::default();
    let (id, rx) = hub.register().unwrap();
    hub.answer(id, answer(json!({"kind": "text", "value": "root"})))
        .unwrap();
    let got = timeout(WAIT, rx).await.unwrap().unwrap();
    assert!(matches!(got, PromptAnswer::Text { value } if value == "root"));
    let (second, _rx) = hub.register().unwrap();
    assert_ne!(id, second, "prompt ids are unique");
}

#[test]
fn answer_for_an_unknown_prompt_id_is_not_found() {
    let hub = PromptHub::default();
    let (id, _rx) = hub.register().unwrap();
    let err = hub
        .answer(id + 1, answer(json!({"kind": "cancel"})))
        .unwrap_err();
    assert_eq!(err.code, "not_found");
    hub.answer(id, answer(json!({"kind": "cancel"}))).unwrap();
    // answered once: a second answer for the same prompt finds nothing
    assert_eq!(
        code(hub.answer(id, answer(json!({"kind": "cancel"})))),
        "not_found"
    );
}

#[tokio::test]
async fn dropping_the_hub_resolves_every_waiting_ask_with_none() {
    let hub = PromptHub::default();
    let (_, a) = hub.register().unwrap();
    let (_, b) = hub.register().unwrap();
    drop(hub);
    assert!(timeout(WAIT, a).await.unwrap().is_err());
    assert!(timeout(WAIT, b).await.unwrap().is_err());
}

#[tokio::test]
async fn closing_the_hub_resolves_waiting_asks_and_refuses_new_ones() {
    let hub = PromptHub::default();
    let (_, a) = hub.register().unwrap();
    hub.close();
    assert!(timeout(WAIT, a).await.unwrap().is_err());
    assert!(hub.register().is_none());
}

fn interaction() -> (TauriInteraction, Rec) {
    let (ch, rec) = recorder();
    (
        TauriInteraction {
            hub: Arc::new(PromptHub::default()),
            sink: Arc::new(Sink::new(ch)),
        },
        rec,
    )
}

#[tokio::test]
async fn interaction_maps_answers_and_falls_back_to_the_safe_answer() {
    use fernjo_ssh::{
        HostKeyChangedDecision, HostKeyChangedPrompt, HostKeyDecision, HostKeyPrompt, Interaction,
        KiPrompt, KiRound,
    };
    use secrecy::ExposeSecret;
    let (ia, mut rec) = interaction();
    let hub = Arc::clone(&ia.hub);
    let hk = || HostKeyPrompt {
        host: "h".into(),
        port: 22,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:x".into(),
    };
    let reply = |a: Value| {
        let hub = Arc::clone(&hub);
        // the prompt registers before it is emitted, so the next id is known
        move |id: u64| hub.answer(id, answer(a)).unwrap()
    };
    // decisions map one to one
    let (d, _) = tokio::join!(ia.host_key(hk()), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "hostKey", "decision": "always"}))(id);
    });
    assert_eq!(d, HostKeyDecision::TrustAlways);
    // a wrong answer shape or a cancel is the safe answer
    let (d, _) = tokio::join!(ia.host_key(hk()), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "secrets", "values": ["x"]}))(id);
    });
    assert_eq!(d, HostKeyDecision::Reject);
    let changed = HostKeyChangedPrompt {
        host: "h".into(),
        port: 22,
        key_type: "ssh-ed25519".into(),
        old: vec!["SHA256:a".into()],
        new: "SHA256:b".into(),
    };
    let (d, _) = tokio::join!(ia.host_key_changed(changed), async {
        let (id, p) = rec.wait_prompt().await;
        assert_eq!(p["kind"], "hostKeyChanged");
        reply(json!({"kind": "cancel"}))(id);
    });
    assert_eq!(d, HostKeyChangedDecision::Reject);
    // typed secrets reach the caller as secrets; the wrong count is a cancel
    let (p, _) = tokio::join!(ia.password("u", "h"), async {
        let (id, p) = rec.wait_prompt().await;
        assert_eq!(p, json!({"kind": "password", "user": "u", "host": "h"}));
        reply(json!({"kind": "secrets", "values": ["pw"]}))(id);
    });
    assert_eq!(p.unwrap().expose_secret(), "pw");
    let round = KiRound {
        name: "n".into(),
        instructions: "i".into(),
        prompts: vec![
            KiPrompt {
                text: "a".into(),
                echo: true,
            },
            KiPrompt {
                text: "b".into(),
                echo: false,
            },
        ],
    };
    let (k, _) = tokio::join!(ia.keyboard_interactive(round), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "secrets", "values": ["only-one"]}))(id);
    });
    assert!(k.is_none());
    let (u, _) = tokio::join!(ia.username("h"), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "text", "value": "  root "}))(id);
    });
    assert_eq!(u.as_deref(), Some("root"));
    let (u, _) = tokio::join!(ia.username("h"), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "text", "value": "   "}))(id);
    });
    assert_eq!(u, None);
    let (k, _) = tokio::join!(ia.key_passphrase(), async {
        let (id, _) = rec.wait_prompt().await;
        reply(json!({"kind": "cancel"}))(id);
    });
    assert!(k.is_none());
    // a closed hub answers every ask with the safe answer at once, without a prompt
    hub.close();
    assert_eq!(ia.host_key(hk()).await, HostKeyDecision::Reject);
    assert!(ia.password("u", "h").await.is_none());
}

#[tokio::test]
async fn a_prompt_open_when_the_hub_closes_gets_the_safe_answer() {
    use fernjo_ssh::{HostKeyDecision, HostKeyPrompt, Interaction};
    let (ia, mut rec) = interaction();
    let hub = Arc::clone(&ia.hub);
    let asked = ia.host_key(HostKeyPrompt {
        host: "h".into(),
        port: 22,
        key_type: "t".into(),
        fingerprint: "f".into(),
    });
    let (d, _) = tokio::join!(asked, async {
        rec.wait_prompt().await;
        hub.close();
    });
    assert_eq!(d, HostKeyDecision::Reject);
}

// ---------- how a session ended ----------

const FIVE: Duration = MIN_AUTO_CLOSE_SESSION;

#[test]
fn classify_end_exit_zero_after_five_seconds_is_clean() {
    let e = classify_end(CloseReason::Exit(Some(0)), FIVE);
    assert!(e.clean);
    assert_eq!(e.exit_status, Some(0));
    assert_eq!(e.text, "The remote shell ended (exit status 0).");
}

#[test]
fn classify_end_exit_zero_within_five_seconds_is_not_clean() {
    let e = classify_end(CloseReason::Exit(Some(0)), Duration::from_millis(4900));
    assert!(!e.clean);
    assert_eq!(e.exit_status, Some(0));
}

#[test]
fn classify_end_nonzero_exit_is_not_clean() {
    let e = classify_end(CloseReason::Exit(Some(3)), Duration::from_secs(60));
    assert!(!e.clean);
    assert_eq!(e.exit_status, Some(3));
}

#[test]
fn classify_end_without_status_is_not_clean() {
    let e = classify_end(CloseReason::Exit(None), Duration::from_secs(60));
    assert!(!e.clean);
    assert_eq!(e.exit_status, None);
    assert_eq!(e.text, "The remote shell ended.");
}

#[test]
fn classify_end_connection_lost_is_not_clean() {
    let e = classify_end(CloseReason::ConnectionLost, Duration::from_secs(60));
    assert!(!e.clean);
    assert_eq!(e.exit_status, None);
    assert_eq!(e.text, "The connection was lost.");
}

#[test]
fn classify_end_closed_by_user_is_not_clean() {
    let e = classify_end(CloseReason::ClosedByUser, Duration::from_secs(60));
    assert!(!e.clean);
    assert_eq!(e.exit_status, None);
}

#[test]
fn state_event_json_carries_exit_status_and_ended_cleanly() {
    let ended = UiEvent::State {
        state: ConnState::Disconnected,
        detail: Some("The remote shell ended (exit status 0).".into()),
        exit_status: Some(0),
        ended_cleanly: true,
    };
    assert_eq!(
        serde_json::to_value(&ended).unwrap(),
        json!({"kind": "state", "state": "disconnected",
               "detail": "The remote shell ended (exit status 0).",
               "exitStatus": 0, "endedCleanly": true})
    );
    let connected = UiEvent::State {
        state: ConnState::Connected,
        detail: None,
        exit_status: None,
        ended_cleanly: false,
    };
    assert_eq!(
        serde_json::to_value(&connected).unwrap(),
        json!({"kind": "state", "state": "connected", "detail": null,
               "exitStatus": null, "endedCleanly": false})
    );
}

#[test]
fn session_event_json_has_no_secret_field() {
    let events = [
        UiEvent::State {
            state: ConnState::Connecting,
            detail: None,
            exit_status: None,
            ended_cleanly: false,
        },
        UiEvent::State {
            state: ConnState::NeedsVault,
            detail: None,
            exit_status: None,
            ended_cleanly: false,
        },
        UiEvent::State {
            state: ConnState::Connected,
            detail: None,
            exit_status: None,
            ended_cleanly: false,
        },
        UiEvent::State {
            state: ConnState::Disconnected,
            detail: Some("gone".into()),
            exit_status: None,
            ended_cleanly: false,
        },
        UiEvent::Prompt {
            id: 1,
            prompt: PromptView::HostKey {
                host: "h".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                fingerprint: "SHA256:x".into(),
            },
        },
        UiEvent::Prompt {
            id: 2,
            prompt: PromptView::HostKeyChanged {
                host: "h".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                old: vec!["SHA256:a".into()],
                new: "SHA256:b".into(),
            },
        },
        UiEvent::Prompt {
            id: 3,
            prompt: PromptView::KeyboardInteractive {
                name: "n".into(),
                instructions: "i".into(),
                prompts: vec![KiPromptView {
                    text: "Code: ".into(),
                    echo: false,
                }],
            },
        },
        UiEvent::Prompt {
            id: 4,
            prompt: PromptView::Password {
                user: "u".into(),
                host: "h".into(),
            },
        },
        UiEvent::Prompt {
            id: 5,
            prompt: PromptView::Username { host: "h".into() },
        },
        UiEvent::Prompt {
            id: 6,
            prompt: PromptView::KeyPassphrase {},
        },
        UiEvent::Prompt {
            id: 7,
            prompt: PromptView::HostKeySynced {
                host: "h".into(),
                port: 22,
                key_type: "ssh-ed25519".into(),
                fingerprint: "SHA256:x".into(),
                previous: vec!["SHA256:a".into()],
            },
        },
    ];
    let json: Vec<Value> = events
        .iter()
        .map(|e| serde_json::to_value(e).unwrap())
        .collect();
    assert_eq!(
        json[0],
        json!({"kind": "state", "state": "connecting", "detail": null,
               "exitStatus": null, "endedCleanly": false})
    );
    assert_eq!(json[1]["state"], "needsVault");
    assert_eq!(json[3]["detail"], "gone");
    assert_eq!(
        json[4],
        json!({"kind": "prompt", "id": 1, "prompt": {"kind": "hostKey", "host": "h", "port": 22,
            "keyType": "ssh-ed25519", "fingerprint": "SHA256:x"}})
    );
    assert_eq!(json[5]["prompt"]["kind"], "hostKeyChanged");
    assert_eq!(json[5]["prompt"]["old"], json!(["SHA256:a"]));
    assert_eq!(json[5]["prompt"]["new"], "SHA256:b");
    assert_eq!(json[6]["prompt"]["prompts"][0]["echo"], false);
    assert_eq!(
        json[7]["prompt"],
        json!({"kind": "password", "user": "u", "host": "h"})
    );
    assert_eq!(json[8]["prompt"], json!({"kind": "username", "host": "h"}));
    assert_eq!(json[9]["prompt"], json!({"kind": "keyPassphrase"}));
    assert_eq!(
        json[10]["prompt"],
        json!({"kind": "hostKeySynced", "host": "h", "port": 22, "keyType": "ssh-ed25519",
            "fingerprint": "SHA256:x", "previous": ["SHA256:a"]})
    );
    // no key anywhere in the serialised events names a secret
    fn keys(v: &Value, out: &mut Vec<String>) {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    out.push(k.clone());
                    keys(v, out);
                }
            }
            Value::Array(a) => a.iter().for_each(|v| keys(v, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    json.iter().for_each(|v| keys(v, &mut all));
    for k in all {
        let k = k.to_lowercase();
        assert!(
            !k.contains("secret") && !k.contains("value") && k != "answer",
            "{k}"
        );
    }
}

#[test]
fn answers_deserialise_from_the_wire_shapes() {
    assert!(matches!(
        answer(json!({"kind": "hostKey", "decision": "always"})),
        PromptAnswer::HostKey {
            decision: HostKeyAnswer::Always
        }
    ));
    assert!(matches!(
        answer(json!({"kind": "hostKeyChanged", "decision": "replace"})),
        PromptAnswer::HostKeyChanged {
            decision: HostKeyChangedAnswer::Replace
        }
    ));
    assert!(matches!(
        answer(json!({"kind": "secrets", "values": ["a", "b"]})),
        PromptAnswer::Secrets { values } if values.len() == 2
    ));
    assert!(matches!(
        answer(json!({"kind": "cancel"})),
        PromptAnswer::Cancel {}
    ));
    assert!(
        serde_json::from_value::<PromptAnswer>(json!({"kind": "hostKey", "decision": "maybe"}))
            .is_err()
    );
}

#[test]
fn answer_debug_is_redacted() {
    const S: &str = "SENTINEL-typed-secret";
    let secrets = answer(json!({"kind": "secrets", "values": [S, S]}));
    let text = answer(json!({"kind": "text", "value": S}));
    for a in [&secrets, &text] {
        assert!(!format!("{a:?}").contains(S));
        assert!(!format!("{a:#?}").contains(S));
    }
    assert!(!format!("{:?}", Some(&secrets)).contains(S));
}

#[test]
fn open_external_url_accepts_only_http_and_https() {
    for ok in [
        "http://example.com",
        "https://example.com/a?b=c#d",
        "HTTPS://EXAMPLE.COM",
    ] {
        assert!(check_external_url(ok).is_ok(), "{ok}");
    }
    // what is opened is the parsed, normalised URL, not the input text
    for (input, normalised) in [
        (" http://x", "http://x/"),
        (
            "http://x 
",
            "http://x/",
        ),
        ("http:/x", "http://x/"),
        ("http:x", "http://x/"),
        ("ht	tp://x", "http://x/"),
        ("HTTP://X", "http://x/"),
        ("https://Example.com/a b", "https://example.com/a%20b"),
    ] {
        let parsed = check_external_url(input).unwrap_or_else(|_| panic!("{input:?}"));
        assert_eq!(parsed.as_str(), normalised, "{input:?}");
    }
    for bad in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ssh://host",
        "mailto:a@b.c",
        "data:text/html,x",
        "example.com",
        "",
        "https://",
        "http:",
        "http://",
        "http:///",
        "https://:80",
        "ms-settings:",
        // UNC paths: a webview-supplied string must never reach the shell as a file share
        r"\\server\share\file",
        "//server/share/file",
    ] {
        assert_eq!(
            code(check_external_url(bad)),
            "invalid_url",
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn batching_constants_match_the_contract() {
    assert_eq!(MAX_BATCH, 32 * 1024);
    assert_eq!(BATCH_INTERVAL, Duration::from_millis(16));
}

// ---------- state machine without a server ----------

fn recorder() -> (Channel<InvokeResponseBody>, Rec) {
    let (tx, rx) = unbounded_channel();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        let _ = tx.send(body);
        Ok(())
    });
    (
        channel,
        Rec {
            rx,
            seen: Vec::new(),
            output: Vec::new(),
        },
    )
}

struct Rec {
    rx: UnboundedReceiver<InvokeResponseBody>,
    /// Every JSON event received so far, in order.
    seen: Vec<Value>,
    /// Output bytes received and not yet consumed.
    output: Vec<u8>,
}

impl Rec {
    /// The next message (`Null` for output bytes, which land in `output`); `None` after
    /// `within` without one.
    async fn next(&mut self, within: Duration) -> Option<Value> {
        let msg = timeout(within, self.rx.recv()).await.ok()??;
        match msg {
            InvokeResponseBody::Json(text) => {
                let v: Value = serde_json::from_str(&text).unwrap();
                self.seen.push(v.clone());
                Some(v)
            }
            InvokeResponseBody::Raw(bytes) => {
                self.output.extend(bytes);
                Some(Value::Null)
            }
        }
    }

    async fn wait_state(&mut self, name: &str) -> Option<String> {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let ev = self
                .next(left)
                .await
                .unwrap_or_else(|| panic!("no state {name}; seen {:?}", self.seen));
            if ev["kind"] == "state" && ev["state"] == name {
                return ev["detail"].as_str().map(str::to_string);
            }
        }
    }

    async fn wait_prompt(&mut self) -> (u64, Value) {
        let deadline = Instant::now() + WAIT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let ev = self
                .next(left)
                .await
                .unwrap_or_else(|| panic!("no prompt; seen {:?}", self.seen));
            if ev["kind"] == "prompt" {
                return (ev["id"].as_u64().unwrap(), ev["prompt"].clone());
            }
        }
    }

    fn prompt_kinds(&self) -> Vec<String> {
        self.seen
            .iter()
            .filter(|e| e["kind"] == "prompt")
            .map(|e| e["prompt"]["kind"].as_str().unwrap().to_string())
            .collect()
    }

    /// Reads output (acknowledging it) until it contains `needle`.
    async fn read_until(&mut self, st: &AppState, id: &str, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        let mut text = String::new();
        loop {
            if !self.output.is_empty() {
                let bytes = std::mem::take(&mut self.output);
                session_ack(st, id, bytes.len()).unwrap();
                text.push_str(&String::from_utf8_lossy(&bytes));
            }
            if text.contains(needle) {
                return text;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            self.next(left)
                .await
                .unwrap_or_else(|| panic!("no {needle:?} in {text:?}; seen {:?}", self.seen));
        }
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    st: AppState,
    node: fernjo_core::NodeId,
    known_hosts: PathBuf,
}

/// A state with an unlocked vault holding one password entry (for `user`), and a connection to
/// `host:port` that references it. `locked` locks the vault again.
fn rig(host: &str, port: u32, user: &str, password: &str, locked: bool) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let st = AppState::open(Some(dir.path().to_path_buf()));
    vault_create(&st, SecretString::from(MASTER), false).unwrap();
    let entry = create_credential(
        &st,
        serde_json::from_value(json!({
            "name": "pw", "kind": "password", "username": user, "password": password
        }))
        .unwrap(),
    )
    .unwrap()
    .id;
    let node = commands::create_connection(&st, None, None, "c".into(), Protocol::Ssh, host.into())
        .unwrap()
        .id
        .unwrap();
    commands::update_node(
        &st,
        node,
        serde_json::from_value(json!({"credential": entry, "port": port})).unwrap(),
    )
    .unwrap();
    if locked {
        vault_lock(&st).unwrap();
    }
    Rig {
        known_hosts: dir.path().join("known_hosts"),
        _dir: dir,
        st,
        node,
    }
}

#[tokio::test]
async fn open_session_errors_for_missing_and_non_connection_nodes() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (ch, _rec) = recorder();
    let folder = commands::create_folder(&r.st, None, None, "f".into())
        .unwrap()
        .id
        .unwrap();
    assert_eq!(
        code(open_session(&r.st, folder, ch, 80, 24)),
        "not_a_connection"
    );
    let (ch, _rec) = recorder();
    let missing = fernjo_core::NodeId::new_v4();
    assert_eq!(code(open_session(&r.st, missing, ch, 80, 24)), "not_found");
}

#[tokio::test]
async fn unknown_session_ids_are_not_found_for_every_command() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let id = "no-such-session";
    assert_eq!(code(session_input(&r.st, id, b"x".to_vec())), "not_found");
    assert_eq!(code(session_resize(&r.st, id, 80, 24)), "not_found");
    assert_eq!(code(session_ack(&r.st, id, 1)), "not_found");
    assert_eq!(code(close_session(&r.st, id)), "not_found");
    assert_eq!(code(reconnect_session(&r.st, id, 80, 24)), "not_found");
    assert_eq!(code(resume_session(&r.st, id, false)), "not_found");
    assert_eq!(
        code(answer_prompt(
            &r.st,
            id,
            1,
            answer(json!({"kind": "cancel"}))
        )),
        "not_found"
    );
}

#[tokio::test]
async fn locked_vault_waits_for_resume_and_gates_reconnect_and_close_removes() {
    let r = rig("127.0.0.1", 1, "u", "p", true);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    assert!(uuid::Uuid::parse_str(&id).is_ok(), "the id is a UUID");
    assert_eq!(rec.wait_state("needsVault").await, None);
    // not disconnected: reconnect is refused; input has no shell yet
    assert_eq!(
        code(reconnect_session(&r.st, &id, 80, 24)),
        "session_active"
    );
    assert_eq!(
        code(session_input(&r.st, &id, b"x".to_vec())),
        "session_not_connected"
    );
    // resume while the vault is still locked asks for the vault again
    resume_session(&r.st, &id, false).unwrap();
    assert_eq!(rec.wait_state("needsVault").await, None);
    // unknown prompt id
    assert_eq!(
        code(answer_prompt(
            &r.st,
            &id,
            99,
            answer(json!({"kind": "cancel"}))
        )),
        "not_found"
    );
    close_session(&r.st, &id).unwrap();
    assert_eq!(code(close_session(&r.st, &id)), "not_found");
    assert_eq!(code(resume_session(&r.st, &id, false)), "not_found");
}

#[tokio::test]
async fn resume_after_unlock_with_nothing_listening_ends_disconnected() {
    // port 1 refuses at once: proves resume starts a connect with the unlocked credential and
    // that a failed connect is reported as `disconnected` with the error sentence
    let r = rig("127.0.0.1", 1, "u", "p", true);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    rec.wait_state("needsVault").await;
    vault_unlock(&r.st, SecretString::from(MASTER), false).unwrap();
    resume_session(&r.st, &id, false).unwrap();
    assert_eq!(rec.wait_state("connecting").await, None);
    let detail = rec.wait_state("disconnected").await.unwrap();
    assert_eq!(detail, "Could not connect to the host.");
    // resume is only for sessions waiting for the vault
    assert_eq!(code(resume_session(&r.st, &id, false)), "session_active");
    // and now reconnect is allowed
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    rec.wait_state("connecting").await;
    rec.wait_state("disconnected").await;
}

fn refusing_quick_target() -> fernjo_core::QuickTarget {
    fernjo_core::parse_quick("u@127.0.0.1:1").unwrap()
}

#[tokio::test]
async fn reconnect_of_a_quick_session_resolves_the_same_target_again() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (ch, mut rec) = recorder();
    let id =
        crate::session_commands::open_quick_session(&r.st, refusing_quick_target(), ch, 80, 24)
            .unwrap();
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some("Could not connect to the host.")
    );
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    assert_eq!(rec.wait_state("connecting").await, None);
    // the quick target is not "gone": the same attempt fails the same way
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some("Could not connect to the host.")
    );
}

#[tokio::test]
async fn a_quick_session_never_touches_the_tree() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let file = r._dir.path().join("connections.json");
    let before = std::fs::read(&file).unwrap();
    let tree = serde_json::to_string(&commands::list_tree(&r.st).unwrap()).unwrap();
    let (ch, mut rec) = recorder();
    let id =
        crate::session_commands::open_quick_session(&r.st, refusing_quick_target(), ch, 80, 24)
            .unwrap();
    rec.wait_state("disconnected").await;
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    rec.wait_state("disconnected").await;
    close_session(&r.st, &id).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), before);
    assert_eq!(
        serde_json::to_string(&commands::list_tree(&r.st).unwrap()).unwrap(),
        tree
    );
    assert!(!r.known_hosts.exists());
}

#[tokio::test]
async fn an_invalid_quick_host_is_refused_before_a_session_exists() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (ch, _rec) = recorder();
    let mut target = refusing_quick_target();
    target.host = "bad host".into();
    assert_eq!(
        code(crate::session_commands::open_quick_session(
            &r.st, target, ch, 80, 24
        )),
        "invalid_host"
    );
}

#[tokio::test]
async fn a_malformed_hand_built_quick_target_opens_no_session() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let bad_port = quick_target_with(|t| t.port = 0);
    let bad_user = quick_target_with(|t| t.user = Some("u\nx".into()));
    let long_user = quick_target_with(|t| t.user = Some("u".repeat(300)));
    for (target, expected) in [
        (bad_port, "quick_bad_port"),
        (bad_user, "quick_bad_target"),
        (long_user, "quick_bad_target"),
    ] {
        let (ch, _rec) = recorder();
        assert_eq!(
            code(crate::session_commands::open_quick_session(
                &r.st, target, ch, 80, 24
            )),
            expected
        );
    }
    assert_eq!(r.st.sessions.connected_count(), 0);
    assert!(r.st.sessions.entries.lock().unwrap().is_empty());
}

fn quick_target_with(f: fn(&mut fernjo_core::QuickTarget)) -> fernjo_core::QuickTarget {
    let mut t = refusing_quick_target();
    f(&mut t);
    t
}

#[tokio::test]
async fn reconnect_reports_a_deleted_node() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    rec.wait_state("disconnected").await;
    commands::delete_node(&r.st, r.node).unwrap();
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some("the connection no longer exists")
    );
    assert!(!r.known_hosts.exists());
}

// ---------- against the OpenSSH container ----------

struct Srv {
    host: String,
    port: u32,
    user: String,
    password: String,
}

/// `None` (with a message) without a configured server; panics when `CI` is set. Hides the SSH
/// agent from this process (it holds the authorised key and would log in without the vault).
fn server() -> Option<Srv> {
    static HIDE_AGENT: std::sync::Once = std::sync::Once::new();
    HIDE_AGENT.call_once(|| {
        // SAFETY: runs once before any test of this process connects anywhere.
        unsafe { std::env::remove_var("SSH_AUTH_SOCK") };
    });
    let Ok(addr) = std::env::var("FERNJO_TEST_SSH_ADDR") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "FERNJO_TEST_SSH_ADDR is not set but CI is: the SSH test server must be running"
        );
        eprintln!("skipped: FERNJO_TEST_SSH_ADDR is not set (tools/ssh-test-server.sh start)");
        return None;
    };
    let (host, port) = addr.rsplit_once(':').unwrap();
    Some(Srv {
        host: host.to_string(),
        port: port.parse().unwrap(),
        user: std::env::var("FERNJO_TEST_SSH_USER").unwrap(),
        password: std::env::var("FERNJO_TEST_SSH_PASSWORD").unwrap(),
    })
}

fn rig_for(s: &Srv, locked: bool) -> Rig {
    rig(&s.host, s.port, &s.user, &s.password, locked)
}

/// Answers prompts until `connected`: trust the host key (`always` when asked to), type the
/// user name and password when the vault did not provide them.
async fn drive(r: &Rig, rec: &mut Rec, id: &str, s: &Srv, always: bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let ev = rec
            .next(left)
            .await
            .unwrap_or_else(|| panic!("never connected; seen {:?}", rec.seen));
        if ev["kind"] == "state" && ev["state"] == "connected" {
            return;
        }
        if ev["kind"] == "state" && ev["state"] == "disconnected" {
            panic!("disconnected: {:?}", rec.seen);
        }
        if ev["kind"] != "prompt" {
            continue;
        }
        let pid = ev["id"].as_u64().unwrap();
        let decision = if always { "always" } else { "once" };
        let a = match ev["prompt"]["kind"].as_str().unwrap() {
            "hostKey" | "hostKeySynced" => json!({"kind": "hostKey", "decision": decision}),
            "username" => json!({"kind": "text", "value": s.user}),
            "password" | "keyboardInteractive" => {
                json!({"kind": "secrets", "values": [s.password]})
            }
            other => panic!("unexpected prompt {other}"),
        };
        answer_prompt(&r.st, id, pid, answer(a)).unwrap();
    }
}

#[tokio::test]
async fn open_session_password_round_trip() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    assert_eq!(rec.wait_state("connecting").await, None);
    drive(&r, &mut rec, &id, &s, false).await;
    // the vault credential supplied user and password: only the host key was asked
    assert_eq!(rec.prompt_kinds(), ["hostKey"]);
    session_input(&r.st, &id, b"echo fernjo$((20+22))\n".to_vec()).unwrap();
    let out = rec.read_until(&r.st, &id, "fernjo42").await;
    assert!(!out.is_empty());
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn a_resize_burst_reaches_the_shell_with_the_last_size() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    drive(&r, &mut rec, &id, &s, false).await;
    // dragging the window edge: many sizes in a row, coalesced into few resize ops
    for i in 0..300u32 {
        session_resize(&r.st, &id, 60 + i % 90, 20 + i % 30).unwrap();
    }
    session_resize(&r.st, &id, 123, 45).unwrap();
    session_input(&r.st, &id, b"echo sz$(stty size | tr ' ' x)\n".to_vec()).unwrap();
    let out = rec.read_until(&r.st, &id, "sz45x123").await;
    assert!(!out.is_empty());
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn host_key_prompt_is_emitted_and_answer_continues() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    let (pid, prompt) = rec.wait_prompt().await;
    assert_eq!(prompt["kind"], "hostKey");
    assert_eq!(prompt["host"], s.host.as_str());
    assert_eq!(prompt["port"], s.port);
    assert!(!prompt["keyType"].as_str().unwrap().is_empty());
    assert!(
        prompt["fingerprint"]
            .as_str()
            .unwrap()
            .starts_with("SHA256:")
    );
    assert!(
        !r.known_hosts.exists(),
        "nothing is stored before the answer"
    );
    answer_prompt(
        &r.st,
        &id,
        pid,
        answer(json!({"kind": "hostKey", "decision": "always"})),
    )
    .unwrap();
    rec.wait_state("connected").await;
    let stored = std::fs::read_to_string(&r.known_hosts).unwrap();
    assert!(!stored.trim().is_empty());
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn needs_vault_waits_and_resume_continues() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, true);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    assert_eq!(rec.wait_state("needsVault").await, None);
    assert!(
        rec.prompt_kinds().is_empty(),
        "nothing connects while the vault is locked"
    );
    vault_unlock(&r.st, SecretString::from(MASTER), false).unwrap();
    resume_session(&r.st, &id, false).unwrap();
    assert_eq!(rec.wait_state("connecting").await, None);
    drive(&r, &mut rec, &id, &s, false).await;
    assert_eq!(rec.prompt_kinds(), ["hostKey"]);
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn resume_with_skip_asks_for_the_password_instead() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, true);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    rec.wait_state("needsVault").await;
    resume_session(&r.st, &id, true).unwrap();
    assert_eq!(
        rec.wait_state("connecting").await.as_deref(),
        Some("Vault skipped")
    );
    drive(&r, &mut rec, &id, &s, false).await;
    let kinds = rec.prompt_kinds();
    assert!(kinds.contains(&"username".to_string()), "{kinds:?}");
    assert!(
        kinds.contains(&"password".to_string())
            || kinds.contains(&"keyboardInteractive".to_string()),
        "{kinds:?}"
    );
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn flood_stops_at_high_water_mark_and_ctrl_c_returns_within_one_second() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    drive(&r, &mut rec, &id, &s, true).await;
    session_input(&r.st, &id, b"echo READY$((1+1))\n".to_vec()).unwrap();
    rec.read_until(&r.st, &id, "READY2").await;
    session_input(&r.st, &id, b"yes\n".to_vec()).unwrap();
    // acknowledge nothing for 2 s and count what arrives
    let mut received = 0usize;
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() || rec.next(left).await.is_none() {
            break;
        }
        received += std::mem::take(&mut rec.output).len();
    }
    assert!(
        received >= 64 * 1024,
        "the flood produced only {received} bytes"
    );
    // the contract values (128 KiB mark, 32 KiB batches), not the constants: the test must notice a change
    let limit = 128 * 1024 + 2 * MAX_BATCH;
    assert!(
        received <= limit,
        "received {received} bytes unacknowledged, limit {limit}"
    );
    // now acknowledge as it arrives, interrupt and ask for a marker
    session_ack(&r.st, &id, received).unwrap();
    session_input(&r.st, &id, b"\x03".to_vec()).unwrap();
    session_input(&r.st, &id, b"\necho MARK$((1+2))\n".to_vec()).unwrap();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(1);
    let mut tail = String::new();
    loop {
        if !rec.output.is_empty() {
            let bytes = std::mem::take(&mut rec.output);
            session_ack(&r.st, &id, bytes.len()).unwrap();
            tail.push_str(&String::from_utf8_lossy(&bytes));
            if tail.contains("MARK3") {
                break;
            }
            if tail.len() > 64 {
                let mut cut = tail.len() - 64;
                while !tail.is_char_boundary(cut) {
                    cut += 1;
                }
                tail.drain(..cut);
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            rec.next(left).await.is_some(),
            "no MARK3 within 1 s after Ctrl+C (waited {:?})",
            started.elapsed()
        );
    }
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn close_all_removes_every_session_and_is_silent() {
    let r = rig("127.0.0.1", 1, "u", "p", true);
    let (ch1, mut rec1) = recorder();
    let (ch2, mut rec2) = recorder();
    let id1 = open_session(&r.st, r.node, ch1, 80, 24).unwrap();
    let id2 = open_session(&r.st, r.node, ch2, 80, 24).unwrap();
    assert_eq!(rec1.wait_state("needsVault").await, None);
    assert_eq!(rec2.wait_state("needsVault").await, None);
    assert_eq!(r.st.sessions.len(), 2);
    r.st.sessions.close_all();
    assert_eq!(r.st.sessions.len(), 0);
    assert_eq!(code(close_session(&r.st, &id1)), "not_found");
    assert_eq!(code(close_session(&r.st, &id2)), "not_found");
    // nothing follows on the channels
    assert!(rec1.next(Duration::from_millis(200)).await.is_none());
    // closing nothing is fine
    r.st.sessions.close_all();
}

#[tokio::test]
async fn close_all_aborts_a_connecting_task_and_closes_its_prompts() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    let (pid, prompt) = rec.wait_prompt().await;
    assert_eq!(prompt["kind"], "hostKey");
    let weak = r.st.sessions.weak_shared(&id).unwrap();
    r.st.sessions.close_all();
    assert_eq!(r.st.sessions.len(), 0);
    let deadline = Instant::now() + Duration::from_secs(3);
    while weak.upgrade().is_some() {
        assert!(
            Instant::now() < deadline,
            "the connect task is still running"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!r.known_hosts.exists(), "nothing may be trusted by closing");
    assert_eq!(
        code(answer_prompt(
            &r.st,
            &id,
            pid,
            answer(json!({"kind": "hostKey", "decision": "always"}))
        )),
        "not_found"
    );
}

#[tokio::test]
async fn closing_the_session_while_a_prompt_is_open_ends_connect() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    let (pid, prompt) = rec.wait_prompt().await;
    assert_eq!(prompt["kind"], "hostKey");
    let weak = r.st.sessions.weak_shared(&id).unwrap();
    close_session(&r.st, &id).unwrap();
    // the connect task (the only other owner of the shared state) ends
    let deadline = Instant::now() + Duration::from_secs(3);
    while weak.upgrade().is_some() {
        assert!(
            Instant::now() < deadline,
            "the connect task is still running"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!r.known_hosts.exists(), "nothing may be trusted by closing");
    assert_eq!(
        code(answer_prompt(
            &r.st,
            &id,
            pid,
            answer(json!({"kind": "hostKey", "decision": "always"}))
        )),
        "not_found"
    );
    // closing is silent: no connected or disconnected event follows
    while let Some(ev) = rec.next(Duration::from_millis(300)).await {
        assert!(ev["kind"] != "state", "unexpected {ev:?}");
    }
    assert!(!r.known_hosts.exists());
}

#[tokio::test]
async fn reconnect_uses_current_settings_and_reports_a_deleted_node() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    drive(&r, &mut rec, &id, &s, true).await;
    // the connection is only reconnectable once it is down
    assert_eq!(
        code(reconnect_session(&r.st, &id, 80, 24)),
        "session_active"
    );
    session_input(&r.st, &id, b"exit\n".to_vec()).unwrap();
    rec.wait_state("disconnected").await;
    // point the node at a port nobody listens on: reconnect must use the tree as it is now
    let patch = |port: u32| serde_json::from_value(json!({"port": port})).unwrap();
    commands::update_node(&r.st, r.node, patch(1)).unwrap();
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    rec.wait_state("connecting").await;
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some("Could not connect to the host.")
    );
    // and back: the host key is known now, so it connects without asking again
    commands::update_node(&r.st, r.node, patch(s.port)).unwrap();
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    rec.wait_state("connected").await;
    assert_eq!(rec.prompt_kinds(), ["hostKey"]);
    session_input(&r.st, &id, b"exit\n".to_vec()).unwrap();
    rec.wait_state("disconnected").await;
    commands::delete_node(&r.st, r.node).unwrap();
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some("the connection no longer exists")
    );
    close_session(&r.st, &id).unwrap();
}

#[tokio::test]
async fn server_drop_ends_in_disconnected_with_a_reason() {
    let Some(s) = server() else { return };
    let r = rig_for(&s, false);
    let (ch, mut rec) = recorder();
    let id = open_session(&r.st, r.node, ch, 80, 24).unwrap();
    drive(&r, &mut rec, &id, &s, false).await;
    session_input(&r.st, &id, b"exit 3\n".to_vec()).unwrap();
    let detail = rec.wait_state("disconnected").await.unwrap();
    assert!(detail.contains('3'), "{detail}");
    // the session is down: input is refused
    assert_eq!(
        code(session_input(&r.st, &id, b"x".to_vec())),
        "session_not_connected"
    );
}

#[tokio::test]
async fn synced_host_key_prompt_maps_answers_and_falls_back_to_reject() {
    use fernjo_ssh::{HostKeyDecision, HostKeySyncedPrompt, Interaction};
    let (ia, mut rec) = interaction();
    let hub = Arc::clone(&ia.hub);
    let prompt = || HostKeySyncedPrompt {
        host: "h".into(),
        port: 22,
        key_type: "ssh-ed25519".into(),
        fingerprint: "SHA256:x".into(),
        previous: vec!["SHA256:a".into()],
    };
    for (wire, expected) in [
        (
            json!({"kind": "hostKey", "decision": "always"}),
            HostKeyDecision::TrustAlways,
        ),
        (
            json!({"kind": "hostKey", "decision": "once"}),
            HostKeyDecision::TrustOnce,
        ),
        (
            json!({"kind": "hostKey", "decision": "reject"}),
            HostKeyDecision::Reject,
        ),
        (json!({"kind": "cancel"}), HostKeyDecision::Reject),
        (
            json!({"kind": "hostKeyChanged", "decision": "replace"}),
            HostKeyDecision::Reject,
        ),
    ] {
        let (d, _) = tokio::join!(ia.host_key_synced(prompt()), async {
            let (id, p) = rec.wait_prompt().await;
            assert_eq!(p["kind"], "hostKeySynced");
            assert_eq!(p["previous"], json!(["SHA256:a"]));
            hub.answer(id, answer(wire)).unwrap();
        });
        assert_eq!(d, expected);
    }
    hub.close();
    assert_eq!(ia.host_key_synced(prompt()).await, HostKeyDecision::Reject);
}

#[test]
fn the_confirmed_hosts_layer_is_the_device_file_or_memory_for_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = AppState::open(Some(dir.path().to_path_buf()));
    let mgr = SessionManager::default();
    let none = mgr.host_files(&st).unwrap();
    assert_eq!(none.known_hosts, dir.path().join("known_hosts"));
    assert!(none.confirmed_hosts.is_none() && none.confirmed_in_memory.is_some());
    let device = tempfile::tempdir().unwrap();
    st.device_dir = Some(device.path().to_path_buf());
    let some = mgr.host_files(&st).unwrap();
    assert_eq!(
        some.confirmed_hosts,
        Some(device.path().join("known_hosts.confirmed"))
    );
    assert!(some.confirmed_in_memory.is_none());
    // The synced data folder never holds the confirmed file.
    assert!(!some.confirmed_hosts.unwrap().starts_with(dir.path()));
}

// ---------- close confirmation ----------

fn entry_in(phase: Phase) -> Arc<Entry> {
    let (channel, _rec) = recorder();
    Arc::new(Entry {
        target: crate::connect_plan::Target::Node(uuid::Uuid::new_v4()),
        shared: Arc::new(Shared {
            sink: Arc::new(Sink::new(channel)),
            hub: Arc::new(PromptHub::default()),
            flow: Mutex::new(FlowControl::default()),
            wake: Notify::new(),
            phase: Mutex::new(phase),
            size: Mutex::new((80, 24)),
            ops: Mutex::new(None),
            input_budget: InputBudget::default(),
            resize_queued: std::sync::atomic::AtomicBool::new(false),
        }),
        task: Mutex::new(None),
    })
}

#[test]
fn connected_count_counts_only_connected_sessions() {
    let manager = SessionManager::default();
    assert_eq!(manager.connected_count(), 0);
    for (id, phase) in [
        ("a", Phase::Connecting),
        ("b", Phase::NeedsVault),
        ("c", Phase::Connected),
        ("d", Phase::Disconnected),
    ] {
        manager
            .entries
            .lock()
            .unwrap()
            .insert(id.to_string(), entry_in(phase));
    }
    assert_eq!(manager.connected_count(), 1);
}

#[test]
fn a_confirmation_is_needed_only_with_a_connected_session() {
    assert!(!needs_close_confirmation(0));
    assert!(needs_close_confirmation(1));
    assert!(needs_close_confirmation(3));
}

#[test]
fn an_explicit_exit_code_is_never_intercepted() {
    assert_eq!(exit_action(Some(0), 3, false), ExitAction::Allow);
    assert_eq!(exit_action(Some(1), 3, true), ExitAction::Allow);
}

#[test]
fn no_connected_session_allows_the_exit() {
    assert_eq!(exit_action(None, 0, false), ExitAction::Allow);
    assert_eq!(exit_action(None, 0, true), ExitAction::Allow);
}

#[test]
fn a_connected_session_asks() {
    assert_eq!(exit_action(None, 1, false), ExitAction::Ask);
}

#[test]
fn a_second_request_while_the_dialog_is_open_prevents_without_a_second_dialog() {
    assert_eq!(exit_action(None, 2, true), ExitAction::Prevent);
}

// ---------- input queue cap ----------

#[test]
fn input_over_the_cap_is_refused_with_input_queue_full() {
    let manager = SessionManager::default();
    let entry = entry_in(Phase::Connected);
    let (tx, mut ops) = unbounded_channel();
    *entry.shared.ops.lock().unwrap() = Some(tx);
    manager
        .entries
        .lock()
        .unwrap()
        .insert("a".to_string(), Arc::clone(&entry));
    let budget = &entry.shared.input_budget;
    let min = crate::session_flow::MIN_OP_WEIGHT;
    assert!(budget.try_reserve(crate::session_flow::MAX_QUEUED_INPUT - min));
    // a short input weighs the minimum
    manager.input("a", b"abcd".to_vec()).unwrap();
    let refused = manager.input("a", b"e".to_vec()).unwrap_err();
    assert_eq!(refused.code, "input_queue_full");
    assert_eq!(refused.message, "too much input is waiting; try again");
    // the session stays alive: the phase is unchanged and what was queued is still there
    assert_eq!(entry.shared.phase().unwrap(), Phase::Connected);
    assert!(matches!(ops.try_recv().unwrap(), Op::Data(d) if d == b"abcd"));
    assert!(ops.try_recv().is_err(), "the refused input was not queued");
    // once the writer released what it handled, input is accepted again
    budget.release(min);
    manager.input("a", b"e".to_vec()).unwrap();
}

#[test]
fn a_closed_queue_gives_the_reservation_back() {
    let manager = SessionManager::default();
    let entry = entry_in(Phase::Connected);
    let (tx, ops) = unbounded_channel();
    *entry.shared.ops.lock().unwrap() = Some(tx);
    manager
        .entries
        .lock()
        .unwrap()
        .insert("a".to_string(), Arc::clone(&entry));
    drop(ops);
    assert_eq!(
        manager.input("a", b"abc".to_vec()).unwrap_err().code,
        "session_not_connected"
    );
    assert_eq!(entry.shared.input_budget.queued(), 0);
}

#[test]
fn unhandled_ops_release_their_budget_when_the_writer_stops() {
    let shared = entry_in(Phase::Connected).shared.clone();
    let (tx, mut ops) = unbounded_channel();
    for op in [
        Op::Data(vec![0; 100]),
        Op::Clipboard("abc".into()),
        Op::Resize,
        Op::Close,
    ] {
        assert!(shared.input_budget.try_reserve(op.weight()));
        tx.send(op).unwrap();
    }
    // 100 bytes, a short text at the minimum weight, resize and close at 0
    assert_eq!(
        shared.input_budget.queued(),
        100 + crate::session_flow::MIN_OP_WEIGHT
    );
    release_unhandled(&mut ops, &shared);
    assert_eq!(shared.input_budget.queued(), 0);
}

#[test]
fn a_flood_of_one_byte_writes_hits_the_cap_by_weight() {
    use crate::session_flow::{MAX_QUEUED_INPUT, MIN_OP_WEIGHT};
    let manager = SessionManager::default();
    let entry = entry_in(Phase::Connected);
    // no writer runs: nothing is ever released
    let (tx, _ops) = unbounded_channel();
    *entry.shared.ops.lock().unwrap() = Some(tx);
    manager
        .entries
        .lock()
        .unwrap()
        .insert("a".to_string(), Arc::clone(&entry));
    for _ in 0..MAX_QUEUED_INPUT / MIN_OP_WEIGHT {
        manager.input("a", b"x".to_vec()).unwrap();
    }
    let refused = manager.input("a", b"x".to_vec()).unwrap_err();
    assert_eq!(refused.code, "input_queue_full");
    assert_eq!(entry.shared.input_budget.queued(), MAX_QUEUED_INPUT);
}

#[test]
fn a_resize_burst_queues_one_op_and_applies_the_last_size() {
    let manager = SessionManager::default();
    let entry = entry_in(Phase::Connected);
    let (tx, mut ops) = unbounded_channel();
    *entry.shared.ops.lock().unwrap() = Some(tx);
    manager
        .entries
        .lock()
        .unwrap()
        .insert("a".to_string(), Arc::clone(&entry));
    // the writer is paused: nothing is taken from the queue
    for i in 0..1_000u32 {
        manager.resize("a", 80 + i, 24 + i).unwrap();
    }
    assert!(matches!(ops.try_recv().unwrap(), Op::Resize));
    assert!(ops.try_recv().is_err(), "a burst queues a single resize");
    // the writer applies the latest size, not the one of the first call
    assert_eq!(take_resize(&entry.shared), (80 + 999, 24 + 999));
    // a resize that arrives while the writer applies the previous one queues a new op, so the
    // final size always reaches the session
    manager.resize("a", 100, 30).unwrap();
    assert!(matches!(ops.try_recv().unwrap(), Op::Resize));
    assert!(ops.try_recv().is_err());
    assert_eq!(take_resize(&entry.shared), (100, 30));
    // resizes never touch the input budget
    assert_eq!(entry.shared.input_budget.queued(), 0);
}

#[test]
fn a_resize_without_a_shell_queues_nothing_and_does_not_block_later_resizes() {
    let manager = SessionManager::default();
    let entry = entry_in(Phase::Disconnected);
    manager
        .entries
        .lock()
        .unwrap()
        .insert("a".to_string(), Arc::clone(&entry));
    manager.resize("a", 90, 30).unwrap();
    assert_eq!(*entry.shared.size.lock().unwrap(), (90, 30));
    // a shell appears (what `install_ops` does at connect)
    let (tx, mut ops) = unbounded_channel();
    install_ops(&entry.shared, tx, |size| size != (90, 30));
    assert!(
        ops.try_recv().is_err(),
        "the size the connect used needs no resize"
    );
    manager.resize("a", 91, 31).unwrap();
    assert!(matches!(ops.try_recv().unwrap(), Op::Resize));
    assert_eq!(take_resize(&entry.shared), (91, 31));
}

#[test]
fn a_resize_during_the_connect_is_queued_when_the_shell_starts() {
    let entry = entry_in(Phase::Connecting);
    // a stale flag from a writer that stopped with a resize still queued
    entry.shared.resize_queued.store(true, Ordering::SeqCst);
    *entry.shared.size.lock().unwrap() = (120, 40);
    let (tx, mut ops) = unbounded_channel();
    install_ops(&entry.shared, tx, |size| size != (80, 24));
    assert!(matches!(ops.try_recv().unwrap(), Op::Resize));
    assert_eq!(take_resize(&entry.shared), (120, 40));
}

// ---------- restored tabs (no server needed) ----------

fn restored(st: &AppState, node: fernjo_core::NodeId) -> (String, Rec) {
    let (ch, rec) = recorder();
    let view = crate::open_tabs::TargetView {
        node: Some(node),
        quick: None,
    };
    let id = crate::session_commands::restore_session(st, view, ch, 80, 24).unwrap();
    (id, rec)
}

#[tokio::test]
async fn a_restored_session_is_disconnected_and_does_not_count_as_connected() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (_id, mut rec) = restored(&r.st, r.node);
    assert_eq!(r.st.sessions.len(), 1);
    assert_eq!(r.st.sessions.connected_count(), 0);
    assert_eq!(
        rec.wait_state("disconnected").await.as_deref(),
        Some(RESTORED_TEXT)
    );
    assert_eq!(rec.seen.len(), 1, "nothing else is sent: {:?}", rec.seen);
}

#[tokio::test]
async fn a_restored_session_never_ends_cleanly() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (_id, mut rec) = restored(&r.st, r.node);
    rec.wait_state("disconnected").await;
    assert_eq!(rec.seen[0]["endedCleanly"], json!(false));
    assert_eq!(rec.seen[0]["exitStatus"], Value::Null);
}

#[tokio::test]
async fn restore_of_a_deleted_node_is_not_found() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let folder = commands::create_folder(&r.st, None, None, "f".into())
        .unwrap()
        .id
        .unwrap();
    let try_restore = |node| {
        let (ch, _rec) = recorder();
        let view = crate::open_tabs::TargetView {
            node: Some(node),
            quick: None,
        };
        code(crate::session_commands::restore_session(
            &r.st, view, ch, 80, 24,
        ))
    };
    assert_eq!(try_restore(fernjo_core::NodeId::new_v4()), "not_found");
    assert_eq!(try_restore(folder), "not_a_connection");
    commands::delete_node(&r.st, r.node).unwrap();
    assert_eq!(try_restore(r.node), "not_found");
    assert_eq!(r.st.sessions.len(), 0);
}

#[tokio::test]
async fn reconnect_of_a_restored_session_connects_with_the_current_plan() {
    let r = rig("127.0.0.1", 1, "u", "p", false);
    let (id, mut rec) = restored(&r.st, r.node);
    rec.wait_state("disconnected").await;
    // the plan is resolved now: with the vault locked the session waits for it
    vault_lock(&r.st).unwrap();
    reconnect_session(&r.st, &id, 80, 24).unwrap();
    rec.wait_state("needsVault").await;
    assert_eq!(r.st.sessions.connected_count(), 0);
    // a quick target restores and connects too (nothing listens on port 1: it ends in an error)
    let (ch, mut quick_rec) = recorder();
    let view = crate::open_tabs::TargetView {
        node: None,
        quick: Some(fernjo_core::parse_quick("user@127.0.0.1:1").unwrap()),
    };
    let quick_id = crate::session_commands::restore_session(&r.st, view, ch, 80, 24).unwrap();
    quick_rec.wait_state("disconnected").await;
    reconnect_session(&r.st, &quick_id, 80, 24).unwrap();
    quick_rec.wait_state("connecting").await;
}

#[tokio::test]
async fn close_all_does_not_touch_the_open_tabs_file() {
    let data = tempfile::tempdir().unwrap();
    let device = tempfile::tempdir().unwrap();
    let st = AppState::open_with_device_dir(data.path().to_path_buf(), device.path().to_path_buf());
    let node = commands::create_connection(&st, None, None, "c".into(), Protocol::Ssh, "h".into())
        .unwrap()
        .id
        .unwrap();
    let tab = crate::open_tabs::SavedTab {
        node: Some(node),
        quick: None,
    };
    crate::open_tabs::save_open_tabs(&st, vec![tab.clone()], Some(0)).unwrap();
    let path = device.path().join(crate::open_tabs::FILE_NAME);
    let before = std::fs::read(&path).unwrap();
    let (_id, mut rec) = restored(&st, node);
    rec.wait_state("disconnected").await;
    st.sessions.close_all();
    assert_eq!(st.sessions.len(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(crate::open_tabs::load(device.path()).tabs, vec![tab]);
}
