use super::*;
use crate::commands;
use crate::session_commands::open_session;
use fernjo_core::Protocol;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::sync::Notify;
use tokio::sync::mpsc::{Sender, UnboundedReceiver, channel};
use tokio::time::timeout;

use super::super::{PromptHub, Sink};
use crate::session_flow::InputBudget;

const WAIT: Duration = Duration::from_secs(15);

fn rect(x: u16, y: u16, width: u16, height: u16) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// What reached the webview, in order: JSON events as values, raw messages as bytes.
#[derive(Debug, Clone, PartialEq)]
enum Msg {
    Json(Value),
    Raw(Vec<u8>),
}

struct Rec {
    rx: UnboundedReceiver<InvokeResponseBody>,
}

impl Rec {
    /// Everything received so far.
    fn drain(&mut self) -> Vec<Msg> {
        let mut out = Vec::new();
        while let Ok(body) = self.rx.try_recv() {
            out.push(match body {
                InvokeResponseBody::Json(text) => Msg::Json(serde_json::from_str(&text).unwrap()),
                InvokeResponseBody::Raw(bytes) => Msg::Raw(bytes),
            });
        }
        out
    }

    async fn next(&mut self) -> Msg {
        let body = timeout(WAIT, self.rx.recv()).await.unwrap().unwrap();
        match body {
            InvokeResponseBody::Json(text) => Msg::Json(serde_json::from_str(&text).unwrap()),
            InvokeResponseBody::Raw(bytes) => Msg::Raw(bytes),
        }
    }
}

fn recorder() -> (Channel<InvokeResponseBody>, Rec) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        let _ = tx.send(body);
        Ok(())
    });
    (channel, Rec { rx })
}

fn shared_with(channel: Channel<InvokeResponseBody>, flow: FlowControl) -> Arc<Shared> {
    Arc::new(Shared {
        sink: Arc::new(Sink::new(channel)),
        hub: Arc::new(PromptHub::default()),
        flow: Mutex::new(flow),
        wake: Notify::new(),
        phase: Mutex::new(Phase::Connected),
        size: Mutex::new((1280, 800)),
        ops: Mutex::new(None),
        closed: std::sync::atomic::AtomicBool::new(false),
        input_budget: InputBudget::default(),
        resize_queued: std::sync::atomic::AtomicBool::new(false),
    })
}

fn entry_with(shared: Arc<Shared>) -> Arc<Entry> {
    Arc::new(Entry {
        target: crate::connect_plan::Target::Node(uuid::Uuid::new_v4()),
        shared,
        task: Mutex::new(None),
    })
}

fn code<T>(r: Result<T, CommandError>) -> String {
    match r {
        Ok(_) => "ok".to_string(),
        Err(e) => e.code,
    }
}

/// A framebuffer whose pixel at (x, y) has the value `(x + 10 * y) % 256` in every byte of the four.
fn patterned(width: u16, height: u16) -> Framebuffer {
    let fb = Framebuffer::new(width, height).unwrap();
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            // wraps for big desktops; the small ones the tests compare by value stay below 256
            let v = u8::try_from((u32::from(x) + 10 * u32::from(y)) % 256).unwrap();
            data.extend_from_slice(&[v, v, v, v]);
        }
    }
    fb.write_region(rect(0, 0, width, height), &data);
    fb
}

// ---------- pure parts ----------

#[test]
fn region_message_header_and_length_are_exact() {
    let fb = patterned(6, 4);
    let message = region_message(&fb, rect(1, 2, 3, 2)).unwrap();
    assert_eq!(message.len(), HEADER_LEN + 3 * 2 * 4);
    assert_eq!(
        &message[..8],
        &[1, 0, 2, 0, 3, 0, 2, 0],
        "x, y, w, h as u16 LE"
    );
    // row 2, columns 1..4 then row 3, columns 1..4
    let expected: Vec<u8> = [21u8, 22, 23, 31, 32, 33]
        .into_iter()
        .flat_map(|v| [v; 4])
        .collect();
    assert_eq!(&message[8..], &expected[..]);
}

#[test]
fn a_header_with_a_large_coordinate_is_little_endian() {
    let fb = Framebuffer::new(2000, 300).unwrap();
    let message = region_message(&fb, rect(1500, 0, 100, 1)).unwrap();
    assert_eq!(&message[..2], &1500u16.to_le_bytes());
    assert_eq!(&message[4..6], &100u16.to_le_bytes());
}

#[test]
fn a_region_is_clipped_to_the_desktop_and_one_outside_is_nothing() {
    let fb = patterned(6, 4);
    let message = region_message(&fb, rect(4, 3, 10, 10)).unwrap();
    assert_eq!(&message[..8], &[4, 0, 3, 0, 2, 0, 1, 0]);
    assert_eq!(message.len(), HEADER_LEN + 2 * 4);
    assert_eq!(region_message(&fb, rect(6, 0, 1, 1)), None);
    assert_eq!(region_message(&fb, rect(0, 4, 1, 1)), None);
}

#[test]
fn damage_keeps_the_union() {
    let mut damage = Damage::default();
    assert!(damage.is_empty());
    damage.add(rect(10, 10, 5, 5));
    damage.add(rect(0, 20, 5, 5));
    assert_eq!(damage.take(), Some(rect(0, 10, 15, 15)));
    assert!(damage.is_empty() && damage.take().is_none());
    damage.add(rect(1, 1, 1, 1));
    damage.clear();
    assert!(damage.is_empty());
}

#[test]
fn sizes_are_even_clamped_and_never_zero() {
    assert_eq!(desktop_size(1281, 801), Some((1280, 800)));
    assert_eq!(desktop_size(1920, 1080), Some((1920, 1080)));
    assert_eq!(desktop_size(10, 5), Some((200, 200)));
    assert_eq!(desktop_size(100_000, 200), Some((8192, 200)));
    assert_eq!(desktop_size(0, 800), None);
    assert_eq!(desktop_size(800, 0), None);
    // one side absurdly large, the other tiny: each side is clamped on its own (8192 x 200 is
    // 1.6 million pixels, far below the area limit, so nothing is scaled)
    assert_eq!(desktop_size(u32::MAX, 1), Some((8192, 200)));
    assert_eq!(desktop_size(1, u32::MAX), Some((200, 8192)));
}

#[test]
fn desktop_size_scales_a_too_large_area_down_keeping_the_aspect() {
    let (w, h) = desktop_size(8192, 8192).unwrap();
    assert!(w % 2 == 0 && h % 2 == 0, "{w}x{h}");
    assert!(
        u32::from(w) * u32::from(h) <= fernjo_rdp::MAX_DESKTOP_PIXELS,
        "{w}x{h}"
    );
    assert!(w.abs_diff(h) <= 2, "{w}x{h}");
    assert!(w > 5000, "scaled down, not collapsed: {w}x{h}");
    assert_eq!(desktop_size(3840, 2160), Some((3840, 2160)));
    // the extremes: no overflow, every result even, in range and within the area
    for (a, b) in [
        (u32::MAX, u32::MAX),
        (u32::MAX, 1),
        (1, u32::MAX),
        (100_000, 9000),
        (8192, 4400),
        (8192, 4321),
        (1, 1),
        (200, 200),
    ] {
        let (w, h) = desktop_size(a, b).unwrap();
        assert!(w % 2 == 0 && h % 2 == 0 && (200..=8192).contains(&w) && (200..=8192).contains(&h));
        assert!(
            u32::from(w) * u32::from(h) <= fernjo_rdp::MAX_DESKTOP_PIXELS,
            "{a}x{b} gave {w}x{h}"
        );
    }
}

#[test]
fn the_credit_stops_above_two_frames_and_resumes_when_one_is_acknowledged() {
    let frame = 100 * 50 * 4;
    let mut flow = credit_for(100, 50);
    assert!(flow.should_send());
    for _ in 0..RDP_FRAMES_IN_FLIGHT {
        assert!(flow.should_send(), "room for {RDP_FRAMES_IN_FLIGHT} frames");
        flow.on_sent(frame);
    }
    assert!(!flow.should_send(), "three whole frames are unacknowledged");
    flow.on_ack(frame);
    assert!(
        flow.should_send(),
        "one acknowledgement refills the pipeline"
    );
}

#[test]
fn rdp_end_is_never_clean() {
    for (reason, text) in [
        (
            CloseReason::ServerClosed,
            "The remote desktop session ended.",
        ),
        (CloseReason::ConnectionLost, "The connection was lost."),
        (CloseReason::ClosedByUser, "The session was closed."),
        (
            CloseReason::ChannelTooLarge,
            "The server sent a channel message that is too large.",
        ),
        (
            CloseReason::UpdateOutOfLimits,
            "The server sent a display update beyond the limits of this client.",
        ),
    ] {
        let ended = classify_rdp_end(reason);
        assert_eq!(ended.text, text);
        assert!(!ended.clean && ended.exit_status.is_none());
    }
}

#[test]
fn a_certificate_rustls_cannot_use_has_its_own_failure_text() {
    // `run` reports a failed connect with the error's text; an unsupported certificate must not
    // read like any other TLS failure
    use fernjo_rdp::RdpError;
    assert_eq!(
        RdpError::CertificateUnsupported.to_string(),
        "the server certificate is not supported"
    );
    assert_eq!(
        RdpError::TlsFailed.to_string(),
        "the secure connection to the server failed"
    );
}

#[test]
fn a_server_without_nla_has_its_own_failure_text() {
    // `run` reports a failed connect with the error's text; a server that is refused for lack of
    // NLA must say why, not read like a protocol error
    use fernjo_rdp::RdpError;
    assert_eq!(RdpError::NlaRequired.code(), "nla_required");
    assert_eq!(
        RdpError::NlaRequired.to_string(),
        "the server does not offer Network Level Authentication (NLA); fernjo only connects to \
         RDP servers with NLA, so the password is only ever sent inside NLA (CredSSP)"
    );
}

// ---------- the forwarder ----------

fn spawn_forward(
    shared: &Arc<Shared>,
    fb: &Framebuffer,
) -> (Sender<SessionEvent>, tokio::task::JoinHandle<Ended>) {
    let (tx, rx) = channel(64);
    let shared = Arc::clone(shared);
    let fb = fb.clone();
    let task = tokio::spawn(async move { forward(&shared, rx, fb).await });
    (tx, task)
}

fn is_desktop(msg: &Msg, width: u64, height: u64) -> bool {
    matches!(msg, Msg::Json(v) if v["kind"] == "desktop" && v["width"] == width && v["height"] == height)
}

#[tokio::test]
async fn desktop_event_precedes_the_first_region() {
    let (channel, mut rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let fb = patterned(8, 8);
    let (tx, task) = spawn_forward(&shared, &fb);
    tx.send(SessionEvent::Damage(rect(0, 0, 8, 8)))
        .await
        .unwrap();
    tx.send(SessionEvent::Closed(CloseReason::ServerClosed))
        .await
        .unwrap();
    let ended = timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(ended.text, "The remote desktop session ended.");
    let seen = rec.drain();
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert!(is_desktop(&seen[0], 8, 8), "{seen:?}");
    assert!(matches!(&seen[1], Msg::Raw(bytes) if bytes.len() == HEADER_LEN + 8 * 8 * 4));
}

#[tokio::test]
async fn damage_is_coalesced_while_the_ui_is_behind() {
    let (channel, mut rec) = recorder();
    // credit for a single byte: the first message pauses the sender
    let shared = shared_with(channel, FlowControl::new(1, 1));
    let fb = patterned(100, 100);
    let (tx, task) = spawn_forward(&shared, &fb);

    tx.send(SessionEvent::Damage(rect(0, 0, 4, 4)))
        .await
        .unwrap();
    assert!(is_desktop(&rec.next().await, 100, 100));
    let first = rec.next().await;
    let Msg::Raw(first) = first else {
        panic!("expected a region")
    };
    assert_eq!(&first[..8], &[0, 0, 0, 0, 4, 0, 4, 0]);

    // the UI is behind: many damage events arrive, none is sent
    for r in [rect(10, 10, 5, 5), rect(50, 20, 5, 5), rect(30, 60, 5, 5)] {
        tx.send(SessionEvent::Damage(r)).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(rec.drain().is_empty(), "nothing is sent without credit");

    // pixels written meanwhile are the ones that go out (the latest win)
    fb.write_region(rect(10, 10, 1, 1), &[250, 251, 252, 253]);
    shared.flow.lock().unwrap().on_ack(first.len());
    shared.wake.notify_one();

    // wait for it before ending the session: a close that wins the race would drop the damage
    let Msg::Raw(second) = rec.next().await else {
        panic!("expected a region")
    };
    tx.send(SessionEvent::Closed(CloseReason::ConnectionLost))
        .await
        .unwrap();
    timeout(WAIT, task).await.unwrap().unwrap();
    let seen = rec.drain();
    assert!(
        seen.is_empty(),
        "exactly one message after the ack: {seen:?}"
    );
    let second = &second;
    // the union of the three rectangles: x 10..55, y 10..65
    assert_eq!(&second[..8], &[10, 0, 10, 0, 45, 0, 55, 0]);
    assert_eq!(second.len(), HEADER_LEN + 45 * 55 * 4);
    assert_eq!(&second[8..12], &[250, 251, 252, 253]);
}

#[tokio::test]
async fn a_desktop_size_change_is_announced_and_drops_the_old_damage() {
    let (channel, mut rec) = recorder();
    let shared = shared_with(channel, FlowControl::new(1, 1));
    let fb = patterned(100, 100);
    let (tx, task) = spawn_forward(&shared, &fb);
    tx.send(SessionEvent::Damage(rect(0, 0, 4, 4)))
        .await
        .unwrap();
    rec.next().await;
    rec.next().await; // desktop, first region; now out of credit
    tx.send(SessionEvent::Damage(rect(0, 0, 100, 100)))
        .await
        .unwrap();
    fb.resize(40, 30).unwrap();
    tx.send(SessionEvent::DesktopSize {
        width: 40,
        height: 30,
    })
    .await
    .unwrap();
    tx.send(SessionEvent::Closed(CloseReason::ServerClosed))
        .await
        .unwrap();
    timeout(WAIT, task).await.unwrap().unwrap();
    let seen = rec.drain();
    assert_eq!(seen.len(), 1, "{seen:?}");
    assert!(is_desktop(&seen[0], 40, 30), "{seen:?}");
}

#[tokio::test]
async fn a_remote_clipboard_event_reaches_the_ui_as_text() {
    let (channel, mut rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let fb = Framebuffer::new(8, 8).unwrap();
    let (tx, task) = spawn_forward(&shared, &fb);
    tx.send(SessionEvent::Clipboard("hello\nworld".into()))
        .await
        .unwrap();
    tx.send(SessionEvent::Closed(CloseReason::ServerClosed))
        .await
        .unwrap();
    timeout(WAIT, task).await.unwrap().unwrap();
    let seen = rec.drain();
    assert_eq!(
        seen[1],
        Msg::Json(json!({"kind": "clipboard", "text": "hello\nworld"}))
    );
}

fn clipboard_texts(seen: &[Msg]) -> Vec<String> {
    seen.iter()
        .filter_map(|msg| match msg {
            Msg::Json(v) if v["kind"] == "clipboard" => Some(v["text"].as_str()?.to_string()),
            _ => None,
        })
        .collect()
}

#[tokio::test(start_paused = true)]
async fn a_burst_of_remote_clipboard_text_reaches_the_ui_coalesced() {
    let (channel, mut rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let fb = Framebuffer::new(8, 8).unwrap();
    let (tx, task) = spawn_forward(&shared, &fb);
    assert!(is_desktop(&rec.next().await, 8, 8));
    let started = tokio::time::Instant::now();

    // a hostile server streams clipboard text; the clock stands still meanwhile
    for i in 0..1000 {
        tx.send(SessionEvent::Clipboard(format!("text {i}")))
            .await
            .unwrap();
    }
    // the forwarder has read all of them by the time the clock moves on by a millisecond
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(
        clipboard_texts(&rec.drain()),
        vec!["text 0".to_string()],
        "the first text goes out at once, the rest waits"
    );

    // after the gap only the latest text follows
    let Msg::Json(next) = rec.next().await else {
        panic!("expected a clipboard event")
    };
    assert_eq!(next["text"], "text 999");
    assert_eq!(started.elapsed(), CLIPBOARD_EVENT_GAP);
    tokio::time::sleep(CLIPBOARD_EVENT_GAP * 4).await;
    assert!(rec.drain().is_empty(), "nothing else was waiting");

    // text still waiting for its gap when the session ends is delivered before the end
    tx.send(SessionEvent::Clipboard("one".into()))
        .await
        .unwrap();
    tx.send(SessionEvent::Clipboard("two".into()))
        .await
        .unwrap();
    tx.send(SessionEvent::Closed(CloseReason::ServerClosed))
        .await
        .unwrap();
    timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(
        clipboard_texts(&rec.drain()),
        vec!["one".to_string(), "two".to_string()]
    );
}

#[tokio::test]
async fn a_closed_event_queue_is_a_lost_connection() {
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let fb = Framebuffer::new(8, 8).unwrap();
    let (tx, task) = spawn_forward(&shared, &fb);
    drop(tx);
    let ended = timeout(WAIT, task).await.unwrap().unwrap();
    assert_eq!(ended.text, "The connection was lost.");
    assert!(!ended.clean);
}

// ---------- the manager ----------

fn manager_with(id: &str, entry: Arc<Entry>) -> SessionManager {
    let manager = SessionManager::default();
    manager
        .entries
        .lock()
        .unwrap()
        .insert(id.to_string(), entry);
    manager
}

fn key(scancode: u8) -> InputEvent {
    InputEvent::Key {
        scancode,
        extended: false,
        pressed: true,
    }
}

#[test]
fn rdp_input_without_a_live_session_is_session_not_connected() {
    let (channel, _rec) = recorder();
    let manager = manager_with("a", entry_with(shared_with(channel, credit_for(8, 8))));
    assert_eq!(
        code(manager.rdp_input("a", vec![key(30)])),
        "session_not_connected"
    );
    assert_eq!(
        code(manager.rdp_clipboard("a", "x".into())),
        "session_not_connected"
    );
    assert_eq!(code(manager.rdp_input("nope", vec![key(30)])), "not_found");
    assert_eq!(code(manager.rdp_clipboard("nope", "x".into())), "not_found");
}

#[test]
fn rdp_input_over_256_events_is_refused_and_256_are_queued() {
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let (tx, mut ops) = tokio::sync::mpsc::unbounded_channel();
    *shared.ops.lock().unwrap() = Some(tx);
    let manager = manager_with("a", entry_with(shared));
    assert_eq!(
        code(manager.rdp_input("a", vec![key(30); 257])),
        "too_many_events"
    );
    assert!(ops.try_recv().is_err(), "a refused batch queues nothing");
    manager.rdp_input("a", vec![key(30); 256]).unwrap();
    manager.rdp_clipboard("a", "text".into()).unwrap();
    match ops.try_recv().unwrap() {
        Op::Rdp(events) => assert_eq!(events.len(), 256),
        _ => panic!("expected input"),
    }
    assert!(matches!(ops.try_recv().unwrap(), Op::Clipboard(t) if t == "text"));
}

#[test]
fn connected_count_counts_rdp_tabs() {
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let manager = manager_with("a", entry_with(Arc::clone(&shared)));
    assert_eq!(manager.connected_count(), 1);
    shared.set_phase(Phase::Disconnected);
    assert_eq!(manager.connected_count(), 0);
}

// ---------- prompts ----------

fn interaction() -> (RdpInteraction, Rec) {
    let (channel, rec) = recorder();
    (
        RdpInteraction(TauriInteraction {
            hub: Arc::new(PromptHub::default()),
            sink: Arc::new(Sink::new(channel)),
        }),
        rec,
    )
}

fn cert_prompt() -> CertPrompt {
    CertPrompt {
        host: "dc01".into(),
        port: 3389,
        fingerprint: "sha256:ab".into(),
    }
}

/// Waits for the prompt event and returns its id and view.
async fn prompt_of(rec: &mut Rec) -> (u64, Value) {
    loop {
        if let Msg::Json(v) = rec.next().await
            && v["kind"] == "prompt"
        {
            return (v["id"].as_u64().unwrap(), v["prompt"].clone());
        }
    }
}

fn answer(v: Value) -> PromptAnswer {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn certificate_prompts_map_answers_and_reject_by_default() {
    let (i, mut rec) = interaction();
    let hub = Arc::clone(&i.0.hub);
    let i = Arc::new(i);
    for (given, want) in [
        (
            json!({"kind": "hostKey", "decision": "once"}),
            CertDecision::TrustOnce,
        ),
        (
            json!({"kind": "hostKey", "decision": "always"}),
            CertDecision::TrustAlways,
        ),
        (
            json!({"kind": "hostKey", "decision": "reject"}),
            CertDecision::Reject,
        ),
        // the wrong shape and a cancel are the safe answer
        (
            json!({"kind": "text", "value": "yes"}),
            CertDecision::Reject,
        ),
        (json!({"kind": "cancel"}), CertDecision::Reject),
    ] {
        let ask = {
            let i = Arc::clone(&i);
            tokio::spawn(async move { i.certificate(cert_prompt()).await })
        };
        let (id, view) = prompt_of(&mut rec).await;
        assert_eq!(
            view,
            json!({"kind": "certificate", "host": "dc01", "port": 3389, "fingerprint": "sha256:ab"})
        );
        hub.answer(id, answer(given)).unwrap();
        assert_eq!(timeout(WAIT, ask).await.unwrap().unwrap(), want);
    }
}

#[tokio::test]
async fn a_changed_certificate_is_replaced_only_on_an_explicit_answer() {
    let (i, mut rec) = interaction();
    let hub = Arc::clone(&i.0.hub);
    let i = Arc::new(i);
    for (given, want) in [
        (
            json!({"kind": "hostKeyChanged", "decision": "replace"}),
            CertChangedDecision::Replace,
        ),
        (
            json!({"kind": "hostKeyChanged", "decision": "reject"}),
            CertChangedDecision::Reject,
        ),
        // an answer meant for the first-time prompt does not replace a pin
        (
            json!({"kind": "hostKey", "decision": "always"}),
            CertChangedDecision::Reject,
        ),
    ] {
        let ask = {
            let i = Arc::clone(&i);
            tokio::spawn(async move {
                i.certificate_changed(CertChangedPrompt {
                    host: "dc01".into(),
                    port: 3389,
                    old: "sha256:aa".into(),
                    new: "sha256:bb".into(),
                })
                .await
            })
        };
        let (id, view) = prompt_of(&mut rec).await;
        assert_eq!(
            view,
            json!({"kind": "certificateChanged", "host": "dc01", "port": 3389,
                   "old": "sha256:aa", "new": "sha256:bb"})
        );
        hub.answer(id, answer(given)).unwrap();
        assert_eq!(timeout(WAIT, ask).await.unwrap().unwrap(), want);
    }
}

#[tokio::test]
async fn username_and_password_prompts_are_the_shared_ones() {
    let (i, mut rec) = interaction();
    let hub = Arc::clone(&i.0.hub);
    let i = Arc::new(i);
    let name = {
        let i = Arc::clone(&i);
        tokio::spawn(async move { i.username("dc01").await })
    };
    let (id, view) = prompt_of(&mut rec).await;
    assert_eq!(view, json!({"kind": "username", "host": "dc01"}));
    hub.answer(id, answer(json!({"kind": "text", "value": " admin "})))
        .unwrap();
    assert_eq!(
        timeout(WAIT, name).await.unwrap().unwrap().as_deref(),
        Some("admin")
    );
    let pw = {
        let i = Arc::clone(&i);
        tokio::spawn(async move { i.password("admin", "dc01").await })
    };
    let (id, view) = prompt_of(&mut rec).await;
    assert_eq!(
        view,
        json!({"kind": "password", "user": "admin", "host": "dc01"})
    );
    hub.answer(id, answer(json!({"kind": "cancel"}))).unwrap();
    assert!(timeout(WAIT, pw).await.unwrap().unwrap().is_none());
}

#[tokio::test]
async fn closing_a_connecting_rdp_session_answers_open_prompts_safely() {
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    // the interaction of the session shares its hub
    let i = Arc::new(RdpInteraction(TauriInteraction {
        hub: Arc::clone(&shared.hub),
        sink: Arc::clone(&shared.sink),
    }));
    let entry = entry_with(Arc::clone(&shared));
    let ask = {
        let i = Arc::clone(&i);
        tokio::spawn(async move { i.certificate(cert_prompt()).await })
    };
    // wait until the prompt is registered, then close the session
    loop {
        if shared
            .hub
            .inner
            .lock()
            .map(|h| !h.waiting.is_empty())
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    super::super::shut_down(&entry);
    assert_eq!(
        timeout(WAIT, ask).await.unwrap().unwrap(),
        CertDecision::Reject
    );
}

// ---------- parameters from the plan ----------

fn plan(resolution: RdpResolution) -> RdpPlan {
    RdpPlan {
        host: "dc01".into(),
        port: 3390,
        username: Some("admin".into()),
        domain: Some("CORP".into()),
        resolution,
        color_depth: 16,
        clipboard: false,
        credential: RdpCredentialPlan::Ready(SecretString::from("sentinel-pw".to_string())),
    }
}

#[test]
fn fit_takes_the_tab_size_and_follows_it_later() {
    use secrecy::ExposeSecret;
    let p = build_params(
        plan(RdpResolution::Fit),
        (1281, 799),
        CertStore::in_memory(),
    );
    assert_eq!((p.width, p.height), (1280, 798));
    assert!(p.dynamic_resize);
    assert_eq!((p.host.as_str(), p.port), ("dc01", 3390));
    assert_eq!(p.username, "admin");
    assert_eq!(p.domain.as_deref(), Some("CORP"));
    assert_eq!(p.color_depth, 16);
    assert!(!p.clipboard);
    assert_eq!(p.password.expose_secret(), "sentinel-pw");
    // a tab that reported nothing yet gets the default size
    let p = build_params(plan(RdpResolution::Fit), (0, 0), CertStore::in_memory());
    assert_eq!((p.width, p.height), (1280, 800));
}

#[test]
fn a_down_level_user_name_is_not_combined_with_a_domain() {
    // sspi refuses `CORP\admin` together with a domain, so the domain is left out then
    let mut p = plan(RdpResolution::Fit);
    p.username = Some("CORP\\admin".into());
    let params = build_params(p, (800, 600), CertStore::in_memory());
    assert_eq!(params.username, "CORP\\admin");
    assert_eq!(params.domain, None);
    // a plain name or a UPN keeps the domain
    for name in ["admin", "admin@corp.example"] {
        let mut p = plan(RdpResolution::Fit);
        p.username = Some(name.into());
        let params = build_params(p, (800, 600), CertStore::in_memory());
        assert_eq!(params.domain.as_deref(), Some("CORP"), "{name}");
    }
}

#[test]
fn an_oversize_clipboard_text_is_not_queued() {
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let (tx, mut ops) = tokio::sync::mpsc::unbounded_channel();
    *shared.ops.lock().unwrap() = Some(tx);
    let manager = manager_with("a", entry_with(shared));
    manager
        .rdp_clipboard("a", "x".repeat(fernjo_rdp::MAX_CLIPBOARD_BYTES + 1))
        .unwrap();
    assert!(ops.try_recv().is_err());
    manager
        .rdp_clipboard("a", "x".repeat(fernjo_rdp::MAX_CLIPBOARD_BYTES))
        .unwrap();
    assert!(matches!(ops.try_recv().unwrap(), Op::Clipboard(_)));
}

#[test]
fn a_fixed_resolution_ignores_the_tab_and_turns_resizing_off() {
    let p = build_params(
        plan(RdpResolution::Fixed {
            width: 1920,
            height: 1080,
        }),
        (500, 400),
        CertStore::in_memory(),
    );
    assert_eq!((p.width, p.height), (1920, 1080));
    assert!(!p.dynamic_resize);
}

#[test]
fn a_missing_credential_connects_with_an_empty_password_so_the_session_asks() {
    use secrecy::ExposeSecret;
    for credential in [
        RdpCredentialPlan::None,
        RdpCredentialPlan::Missing(crate::connect_plan::MissingReason::Skipped),
    ] {
        let mut p = plan(RdpResolution::Fit);
        p.credential = credential;
        p.username = None;
        let params = build_params(p, (800, 600), CertStore::in_memory());
        assert_eq!(params.password.expose_secret(), "");
        assert_eq!(params.username, "");
    }
}

#[test]
fn connect_params_debug_shows_no_password() {
    let p = build_params(plan(RdpResolution::Fit), (800, 600), CertStore::in_memory());
    assert!(!format!("{p:?}").contains("sentinel-pw"));
}

// ---------- the whole path against a closed port ----------

#[tokio::test]
async fn open_session_for_an_rdp_node_starts_an_rdp_session() {
    let dir = tempfile::tempdir().unwrap();
    let st = crate::AppState::open(Some(dir.path().to_path_buf()));
    let node = commands::create_connection(
        &st,
        None,
        None,
        "r".into(),
        Protocol::Rdp,
        "127.0.0.1".into(),
    )
    .unwrap()
    .id
    .unwrap();
    commands::update_node(
        &st,
        node,
        serde_json::from_value(json!({"port": 1})).unwrap(),
    )
    .unwrap();
    let (channel, mut rec) = recorder();
    let id = open_session(&st, node, channel, 1280, 800).unwrap();
    // the settings this attempt uses (M11 review #61: the clipboard marker follows them)
    assert_eq!(
        rec.next().await,
        Msg::Json(json!({"kind": "rdpSettings", "clipboard": true, "host": "127.0.0.1"}))
    );
    // connecting, then the RDP layer's own refusal (an SSH session says something else)
    let first = rec.next().await;
    assert_eq!(
        first,
        Msg::Json(json!({
            "kind": "state", "state": "connecting", "detail": null,
            "exitStatus": null, "endedCleanly": false
        }))
    );
    let Msg::Json(ended) = rec.next().await else {
        panic!("expected the disconnect")
    };
    assert_eq!(ended["state"], "disconnected");
    assert_eq!(ended["detail"], "could not connect to the host");
    assert_eq!(st.sessions.len(), 1);
    // an RDP tab with nothing running refuses input the way a terminal does
    assert_eq!(
        code(st.sessions.rdp_input(&id, vec![key(30)])),
        "session_not_connected"
    );
}

#[tokio::test]
async fn the_rdp_settings_event_carries_the_clipboard_setting_the_session_uses() {
    // M11 review #61: the marker and the UI clipboard sync follow the plan of the attempt, also on
    // a reconnect after the setting changed, never a separate read of the connection
    let dir = tempfile::tempdir().unwrap();
    let st = crate::AppState::open(Some(dir.path().to_path_buf()));
    let node = commands::create_connection(
        &st,
        None,
        None,
        "r".into(),
        Protocol::Rdp,
        "127.0.0.1".into(),
    )
    .unwrap()
    .id
    .unwrap();
    let set = |v: Value| {
        commands::update_node(&st, node, serde_json::from_value(v).unwrap()).unwrap();
    };
    set(json!({"port": 1, "rdpClipboard": false}));
    let (channel, mut rec) = recorder();
    let id = open_session(&st, node, channel, 1280, 800).unwrap();
    assert_eq!(
        rec.next().await,
        Msg::Json(json!({"kind": "rdpSettings", "clipboard": false, "host": "127.0.0.1"}))
    );
    // wait for the refused connect to end
    loop {
        if let Msg::Json(v) = rec.next().await
            && v["state"] == "disconnected"
        {
            break;
        }
    }
    set(json!({"rdpClipboard": true}));
    crate::session_commands::reconnect_session(&st, &id, 1280, 800).unwrap();
    assert_eq!(
        rec.next().await,
        Msg::Json(json!({"kind": "rdpSettings", "clipboard": true, "host": "127.0.0.1"}))
    );
}

#[test]
fn rdp_ops_weigh_at_least_the_minimum_and_resizes_are_coalesced() {
    use crate::session_flow::MIN_OP_WEIGHT;
    let (channel, _rec) = recorder();
    let shared = shared_with(channel, credit_for(8, 8));
    let (tx, mut ops) = tokio::sync::mpsc::unbounded_channel();
    *shared.ops.lock().unwrap() = Some(tx);
    let manager = manager_with("a", entry_with(Arc::clone(&shared)));
    manager.rdp_input("a", vec![key(30)]).unwrap();
    manager.rdp_clipboard("a", "x".into()).unwrap();
    assert_eq!(shared.input_budget.queued(), 2 * MIN_OP_WEIGHT);
    for i in 0..500u32 {
        manager.resize("a", 1000 + i, 700 + i).unwrap();
    }
    assert!(matches!(ops.try_recv().unwrap(), Op::Rdp(_)));
    assert!(matches!(ops.try_recv().unwrap(), Op::Clipboard(_)));
    assert!(matches!(ops.try_recv().unwrap(), Op::Resize));
    assert!(ops.try_recv().is_err(), "a burst queues a single resize");
    assert_eq!(super::super::take_resize(&shared), (1499, 1199));
    assert_eq!(shared.input_budget.queued(), 2 * MIN_OP_WEIGHT);
}

/// `ConnectParams` and `RdpPlan` print the host but neither the account nor the password
/// (M11 review #79: the guard in `vault_commands` covers the plan, this covers the params built
/// from it).
#[test]
fn debug_output_of_plan_and_params_hides_user_domain_and_password() {
    let p = plan(RdpResolution::Fit);
    let plan_text = format!("{p:?}");
    let params = build_params(p, (1280, 800), CertStore::in_memory());
    for text in [plan_text, format!("{params:?}")] {
        assert!(text.contains("dc01"), "the host stays visible: {text}");
        for secret in ["admin", "CORP", "sentinel-pw"] {
            assert!(!text.contains(secret), "Debug leaked {secret}: {text}");
        }
    }
}
