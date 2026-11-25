//! Sync commands and runs against the real hub (`fernjo-hub`, in process, on loopback) with real
//! `AppState`s: one data folder, device folder and memory keychain per device.

use std::cell::{Cell, RefCell};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use fernjo_core::{Node, NodeKind, Protocol, Tree};
use fernjo_hub::{HubStore, config, router, serve};
use fernjo_sync::ports::{LocalChange, LocalState, VaultChange};
use serde_json::{Value, json};
use tauri::ipc::{Channel, InvokeResponseBody};
use tempfile::TempDir;

use super::*;
use crate::commands;
use crate::vault_commands::{
    CredentialDraft, create_credential, list_credentials, vault_create, vault_lock, vault_status,
    vault_unlock,
};

const HUB_SECRET: &str = "SENTINEL-hubsecret-6d2a91";
const WRONG_SECRET: &str = "SENTINEL-wrongsecret-1f77c0";
const MASTER: &str = "SENTINEL-master-sync-4c1b";

struct Hub {
    _dir: TempDir,
    addr: SocketAddr,
}

impl Hub {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

fn start_hub() -> Hub {
    let dir = TempDir::new().unwrap();
    let store = Arc::new(HubStore::open(dir.path()).unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let addr = serve(
                "127.0.0.1:0".parse().unwrap(),
                router(store, config::hash_secret(HUB_SECRET)),
            )
            .await
            .unwrap();
            tx.send(addr).unwrap();
            std::future::pending::<()>().await;
        });
    });
    Hub {
        _dir: dir,
        addr: rx.recv().unwrap(),
    }
}

fn sec(s: &str) -> SecretString {
    SecretString::from(s.to_owned())
}

/// One device: its own data folder, device folder and (memory) keychain.
struct Dev {
    _dir: TempDir,
    st: AppState,
    data: PathBuf,
    device: PathBuf,
}

fn dev() -> Dev {
    let dir = TempDir::new().unwrap();
    let data = dir.path().join("data");
    let device = dir.path().join("device");
    fs::create_dir_all(&data).unwrap();
    let st = AppState::open_with_device_dir(data.clone(), device.clone());
    Dev {
        _dir: dir,
        st,
        data,
        device,
    }
}

fn setup(d: &Dev, hub: &Hub, name: &str) -> SyncStatus {
    sync_setup(&d.st, hub.url(), sec(HUB_SECRET), None, name.to_owned()).unwrap()
}

fn join(d: &Dev, group_dev: &Dev, name: &str) -> SyncStatus {
    let code = sec(&sync_pairing_code(&group_dev.st).unwrap());
    sync_join(&d.st, &code, name.to_owned(), true).unwrap()
}

fn tree(d: &Dev) -> Tree {
    commands::with_store(&d.st, |s| Ok(s.tree().clone())).unwrap()
}

fn folder(d: &Dev, name: &str) {
    commands::create_folder(&d.st, None, None, name.to_owned()).unwrap();
}

fn conn(d: &Dev, name: &str) -> fernjo_core::NodeId {
    commands::create_connection(
        &d.st,
        None,
        None,
        name.to_owned(),
        Protocol::Ssh,
        "h.example".to_owned(),
    )
    .unwrap()
    .id
    .unwrap()
}

fn rename(d: &Dev, id: fernjo_core::NodeId, name: &str) {
    commands::update_node(
        &d.st,
        id,
        serde_json::from_value(json!({ "name": name })).unwrap(),
    )
    .unwrap();
}

fn names(d: &Dev) -> Vec<String> {
    let mut n: Vec<String> = tree(d).children().iter().map(|n| n.name.clone()).collect();
    n.sort();
    n
}

fn credential(d: &Dev, name: &str) {
    let draft: CredentialDraft = serde_json::from_value(
        json!({"name": name, "kind": "password", "username": "root", "password": "pw-1"}),
    )
    .unwrap();
    create_credential(&d.st, draft).unwrap();
}

fn credential_names(d: &Dev) -> Vec<String> {
    let mut n: Vec<String> = list_credentials(&d.st)
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    n.sort();
    n
}

fn vault_file(d: &Dev) -> Vec<u8> {
    fs::read(d.data.join(fernjo_vault::FILE_NAME)).unwrap()
}

fn vault_state(d: &Dev) -> String {
    vault_status(&d.st).unwrap().state.to_owned()
}

fn now(d: &Dev) -> SyncStatus {
    sync_now(&d.st).unwrap()
}

fn state_file(d: &Dev) -> fernjo_sync::state::SyncState {
    StateStore::new(d.device.clone()).load().unwrap().unwrap()
}

fn codes<T>(r: Result<T, CommandError>) -> String {
    match r {
        Ok(_) => "ok".to_owned(),
        Err(e) => e.code,
    }
}

fn shown<T: Serialize + std::fmt::Debug>(v: &T) -> String {
    format!("{}{:?}", serde_json::to_string(v).unwrap(), v)
}

/// A port that runs `hook` once, right before the first `apply` (after the engine read the data
/// and fetched the head): the moment a user edit or a foreign writer can slip in.
struct HookPort<'a> {
    inner: AppPort<'a>,
    hook: RefCell<Option<Box<dyn FnOnce() + 'a>>>,
    applies: Cell<usize>,
}

impl<'a> HookPort<'a> {
    fn new(state: &'a AppState, hook: impl FnOnce() + 'a) -> Self {
        HookPort {
            inner: AppPort(state),
            hook: RefCell::new(Some(Box::new(hook))),
            applies: Cell::new(0),
        }
    }
}

impl LocalPort for HookPort<'_> {
    fn read(&self) -> Result<LocalState, SyncError> {
        self.inner.read()
    }
    fn apply(&self, expected: &LocalState, next: LocalChange) -> Result<(), SyncError> {
        self.applies.set(self.applies.get() + 1);
        let hook = self.hook.borrow_mut().take();
        if let Some(hook) = hook {
            hook();
        }
        self.inner.apply(expected, next)
    }
    fn vault_unlocked(&self) -> bool {
        self.inner.vault_unlocked()
    }
}

// ---- status, setup, secrets ----------------------------------------------------------------

#[test]
fn sync_status_is_off_without_setup() {
    let d = dev();
    let s = sync_status(&d.st).unwrap();
    assert_eq!(s.state, "off");
    assert_eq!(s.pending_changes, 0);
    assert_eq!(s.device_name, None);
    assert_eq!(s.hub_url, None);
    assert!(s.findings.is_empty());
    assert_eq!(s.error, None);
    assert!(!s.running);
    assert_eq!(codes(sync_now(&d.st)), "not_paired");
    assert_eq!(codes(sync_pairing_code(&d.st)), "not_paired");
    assert_eq!(codes(sync_leave(&d.st)), "not_paired");
    assert_eq!(codes(sync_reseed(&d.st)), "not_paired");
    assert_eq!(codes(sync_edit_secret(&d.st, sec("x"))), "not_paired");
}

#[test]
fn the_status_of_a_paired_device_never_builds_a_hub_address() {
    // a device paired with an https hub (state written directly, no hub runs): the status and
    // the pending count use only local data, in debug and in release builds alike
    let d = dev();
    conn(&d, "web");
    StateStore::new(d.device.clone())
        .save(&fernjo_sync::state::SyncState {
            url: "https://hub.example.test".to_owned(),
            ca_pem: None,
            device: fernjo_sync::DeviceInfo {
                id: uuid::Uuid::from_u128(1),
                name: "d".to_owned(),
            },
            group: uuid::Uuid::from_u128(2),
            last_seq: 1,
        })
        .unwrap();
    let s = sync_status(&d.st).unwrap();
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(s.error, None);
    assert_eq!(s.pending_changes, 1, "no base yet: the one node waits");
    assert!(has_pending_changes(&d.st));
}

#[test]
fn setup_without_a_device_folder_or_while_a_restart_waits_is_refused() {
    let hub = start_hub();
    let dir = TempDir::new().unwrap();
    let st = AppState::open(Some(dir.path().to_path_buf()));
    assert_eq!(
        codes(sync_setup(
            &st,
            hub.url(),
            sec(HUB_SECRET),
            None,
            "x".into()
        )),
        "no_device_folder"
    );
    let code = sec("fernjo-sync-1:AAAA");
    assert_eq!(
        codes(sync_join(&st, &code, "x".into(), true)),
        "invalid_pairing_code"
    );
    let d = dev();
    d.st.set_restart_required(true);
    assert_eq!(
        codes(sync_setup(
            &d.st,
            hub.url(),
            sec(HUB_SECRET),
            None,
            "x".into()
        )),
        "restart_required"
    );
    d.st.set_restart_required(false);
    for bad in ["", "   ", "a\nb", &"x".repeat(101)] {
        assert_eq!(
            codes(sync_setup(
                &d.st,
                hub.url(),
                sec(HUB_SECRET),
                None,
                bad.into()
            )),
            "invalid_device_name"
        );
    }
    assert!(!d.device.join("sync.json").exists());
}

#[test]
fn setup_stores_the_secrets_in_the_keychain_and_not_in_any_response() {
    let hub = start_hub();
    let a = dev();
    conn(&a, "web");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut note = |what: &str, text: String| seen.push((what.to_owned(), text));

    // a wrong secret and an unreachable hub fail with a code and echo nothing
    let wrong = dev();
    let e = sync_setup(
        &wrong.st,
        hub.url(),
        sec(WRONG_SECRET),
        None,
        "wrong".into(),
    )
    .unwrap_err();
    assert_eq!(e.code, "hub_unauthorized");
    note("wrong secret", shown(&e));
    let e = sync_setup(
        &wrong.st,
        "http://127.0.0.1:1".into(),
        sec(WRONG_SECRET),
        None,
        "wrong".into(),
    )
    .unwrap_err();
    assert_eq!(e.code, "hub_unreachable");
    note("unreachable", shown(&e));
    assert!(!wrong.device.join("sync.json").exists());

    let status = setup(&a, &hub, "device-a");
    assert_eq!(status.state, "ok");
    assert_eq!(status.device_name.as_deref(), Some("device-a"));
    assert_eq!(status.hub_url, Some(hub.url()));
    assert_eq!(status.pending_changes, 0);
    note("setup", shown(&status));
    let e = sync_setup(&a.st, hub.url(), sec(HUB_SECRET), None, "again".into()).unwrap_err();
    assert_eq!(e.code, "already_paired");
    note("setup again", shown(&e));
    note("status", shown(&sync_status(&a.st).unwrap()));
    note("now", shown(&now(&a)));
    note("AppState Debug", format!("{:?}", a.st));
    note("runtime Debug", format!("{:?}", a.st.sync));

    // positive control: the keychain holds both values
    let group = state_file(&a).group;
    let (stored, key) = a.st.sync_secrets.get(group).unwrap().unwrap();
    assert_eq!(stored.expose_secret(), HUB_SECRET);
    let raw = *key.as_bytes();
    assert_ne!(raw, [0u8; 32]);
    let forbidden = [
        HUB_SECRET.to_owned(),
        WRONG_SECRET.to_owned(),
        base64::engine::general_purpose::STANDARD.encode(raw),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw),
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    ];
    let mut files: Vec<(String, String)> = Vec::new();
    for f in [
        a.device.join("sync.json"),
        a.device.join("sync-base.json"),
        a.data.join(fernjo_core::FILE_NAME),
    ] {
        files.push((
            f.display().to_string(),
            String::from_utf8_lossy(&fs::read(&f).unwrap()).into_owned(),
        ));
    }
    for (what, text) in seen.iter().chain(&files) {
        for s in &forbidden {
            assert!(!text.contains(s), "{what} leaked {s}: {text}");
        }
    }
    // the base holds the encrypted vault bytes, never the master password or a vault secret
    for (what, text) in &files {
        assert!(!text.contains(MASTER), "{what}");
        assert!(!text.contains("pw-1"), "{what}");
    }
}

#[test]
fn pairing_code_is_the_only_response_with_the_secret() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "device-a");
    let code = sync_pairing_code(&a.st).unwrap();
    let decoded = PairingCode::decode(&code).unwrap();
    assert_eq!(decoded.secret.expose_secret(), HUB_SECRET);
    assert_eq!(decoded.url, hub.url());
    assert_eq!(decoded.group, state_file(&a).group);
    let (_, key) = a.st.sync_secrets.get(decoded.group).unwrap().unwrap();
    assert_eq!(decoded.key.as_bytes(), key.as_bytes());
    // the status next to it carries neither
    let status = shown(&sync_status(&a.st).unwrap());
    assert!(!status.contains(HUB_SECRET));
    assert!(!status.contains(&code));
    assert!(!format!("{decoded:?}").contains(HUB_SECRET));
}

// ---- save and offline ----------------------------------------------------------------------

#[test]
fn save_never_waits_for_the_hub() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "device-a");
    // the hub becomes unreachable
    let mut st = state_file(&a);
    st.url = "http://127.0.0.1:1".to_owned();
    StateStore::new(a.device.clone()).save(&st).unwrap();

    let started = Instant::now();
    conn(&a, "while-offline");
    assert!(
        // the point is "not waiting for the hub" (an unreachable hub takes seconds), not fsync speed
        started.elapsed() < Duration::from_secs(2),
        "a save waited for the hub"
    );
    let s = sync_status(&a.st).unwrap();
    assert_eq!(s.pending_changes, 1);
    let s = now(&a);
    assert_eq!(s.state, "offline");
    assert_eq!(s.pending_changes, 1);
    assert_eq!(s.error, None, "an unreachable hub is not an error");
}

#[test]
fn a_close_push_waits_for_a_run_that_holds_the_slot_and_then_pushes() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "a");
    conn(&a, "waiting for the push");
    assert_eq!(sync_status(&a.st).unwrap().pending_changes, 1);

    let guard = try_begin(&a.st).expect("the slot is free");
    let started = Instant::now();
    let released = thread::scope(|scope| {
        let releaser = scope.spawn(move || {
            thread::sleep(Duration::from_millis(200));
            drop(guard);
        });
        let ran = push_before_close_with(&a.st, &AppPort(&a.st), Duration::from_secs(5));
        releaser.join().unwrap();
        ran
    });
    assert!(released, "the push ran once the slot was free");
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "it waited for the other run"
    );
    assert_eq!(sync_status(&a.st).unwrap().pending_changes, 0);
}

#[test]
fn a_close_push_gives_up_when_the_slot_stays_taken_and_skips_when_nothing_waits() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "a");
    // nothing pending: no run, at once
    let started = Instant::now();
    assert!(!push_before_close_with(
        &a.st,
        &AppPort(&a.st),
        Duration::from_secs(5)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));

    conn(&a, "x");
    let _guard = try_begin(&a.st).expect("the slot is free");
    let started = Instant::now();
    assert!(!push_before_close_with(
        &a.st,
        &AppPort(&a.st),
        Duration::from_millis(300)
    ));
    let waited = started.elapsed();
    assert!(
        waited >= Duration::from_millis(250) && waited < Duration::from_secs(3),
        "bounded by the budget, waited {waited:?}"
    );
    assert_eq!(sync_status(&a.st).unwrap().pending_changes, 1);
}

#[test]
fn local_edits_count_for_the_runner_and_sync_writes_do_not() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    let before = a.st.edit_seq.load(Ordering::SeqCst);
    folder(&a, "x");
    assert_eq!(a.st.edit_seq.load(Ordering::SeqCst), before + 1);
    now(&a);
    let seq_b = b.st.edit_seq.load(Ordering::SeqCst);
    now(&b);
    assert_eq!(names(&b), ["x"]);
    assert_eq!(
        b.st.edit_seq.load(Ordering::SeqCst),
        seq_b,
        "an applied sync is not a local edit"
    );
    assert_eq!(
        b.st.sync_applied.load(Ordering::SeqCst),
        2,
        "the join and the merge"
    );
}

// ---- join ----------------------------------------------------------------------------------

#[test]
fn join_without_confirmation_reports_what_would_be_lost_and_changes_nothing() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    folder(&a, "from-a");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");

    folder(&b, "f1");
    conn(&b, "c1");
    conn(&b, "c2");
    vault_create(&b.st, sec(MASTER), false).unwrap();
    credential(&b, "e1");
    credential(&b, "e2");
    let (tree_before, vault_before) = (tree(&b), vault_file(&b));
    let code = sec(&sync_pairing_code(&a.st).unwrap());

    let s = sync_join(&b.st, &code, "b".into(), false).unwrap();
    assert_eq!(s.state, "needs_confirmation");
    assert_eq!(
        s.confirmation,
        Some(JoinConfirmation {
            connections: 2,
            folders: 1,
            vault_entries: Some(2),
            has_vault: true
        })
    );
    assert_eq!(tree(&b), tree_before);
    assert_eq!(vault_file(&b), vault_before);
    assert!(!b.device.join("sync.json").exists());
    assert!(!b.device.join("sync-base.json").exists());
    assert!(
        b.st.sync_secrets
            .get(state_file(&a).group)
            .unwrap()
            .is_none()
    );
    // a locked vault: the entries cannot be counted
    vault_lock(&b.st).unwrap();
    let s = sync_join(&b.st, &code, "b".into(), false).unwrap();
    assert_eq!(s.confirmation.unwrap().vault_entries, None);
    // a damaged code is refused before anything is read
    assert_eq!(
        codes(sync_join(
            &b.st,
            &sec("fernjo-sync-1:xx"),
            "b".into(),
            false
        )),
        "invalid_pairing_code"
    );
}

#[test]
fn join_replaces_tree_and_vault_and_closes_sessions() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    folder(&a, "from-a");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "theirs");
    setup(&a, &hub, "a");

    let node = conn(&b, "mine");
    vault_create(&b.st, sec("SENTINEL-other-master-77"), false).unwrap();
    credential(&b, "mine-entry");
    let own_vault = vault_file(&b);
    let channel = Channel::new(|_: InvokeResponseBody| Ok(()));
    crate::session_commands::open_session(&b.st, node, channel, 80, 24).unwrap();
    assert_eq!(b.st.sessions.len(), 1);

    let s = join(&b, &a, "b");
    assert_eq!(s.state, "ok");
    assert_eq!(s.device_name.as_deref(), Some("b"));
    assert_eq!(b.st.sessions.len(), 0, "the sessions were closed first");
    assert_eq!(tree(&b), tree(&a));
    assert_eq!(names(&b), ["from-a"]);
    assert_eq!(vault_file(&b), vault_file(&a));
    assert_ne!(vault_file(&b), own_vault);
    // another vault id: the handle follows the new file and is locked
    assert_eq!(vault_state(&b), "locked");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    assert_eq!(credential_names(&b), ["theirs"]);
    // the replaced vault stays as the backup file
    assert_eq!(
        fs::read(b.data.join("vault.fernjo.bak")).unwrap(),
        own_vault
    );
    // nothing is waiting, and the device is part of the group now
    assert_eq!(sync_status(&b.st).unwrap().pending_changes, 0);
    let again = sec(&sync_pairing_code(&a.st).unwrap());
    assert_eq!(
        codes(sync_join(&b.st, &again, "b".into(), true)),
        "already_paired"
    );
}

#[test]
fn a_join_that_fails_changes_nothing_and_leaves_no_keychain_item() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    setup(&a, &hub, "a");
    folder(&b, "keep");
    let mut decoded = PairingCode::decode(&sync_pairing_code(&a.st).unwrap()).unwrap();
    // a code for a hub that is not there
    decoded.url = "http://127.0.0.1:1".to_owned();
    let broken = sec(&decoded.encode());
    assert_eq!(
        codes(sync_join(&b.st, &broken, "b".into(), true)),
        "hub_unreachable"
    );
    assert_eq!(names(&b), ["keep"]);
    assert!(!b.device.join("sync.json").exists());
    assert!(b.st.sync_secrets.get(decoded.group).unwrap().is_none());
    assert!(
        !sync_status(&b.st).unwrap().running,
        "the run slot is free again"
    );
}

// ---- runs ----------------------------------------------------------------------------------

#[test]
fn a_local_edit_between_read_and_apply_restarts_the_run() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    let web = conn(&a, "web");
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    // B renames and pushes, so A's next run has something to apply
    rename(&b, web, "web-renamed");
    now(&b);

    // while A's run sits between "read" and "apply" the user adds a folder
    let port = HookPort::new(&a.st, || folder(&a, "typed-meanwhile"));
    assert!(run_background_with(&a.st, &port));
    assert_eq!(
        port.applies.get(),
        2,
        "the first apply refused, the run started over"
    );
    assert_eq!(names(&a), ["typed-meanwhile", "web-renamed"]);
    assert_eq!(sync_status(&a.st).unwrap().state, "ok");
    // the edit survived and reached the hub
    now(&b);
    assert_eq!(names(&b), ["typed-meanwhile", "web-renamed"]);
}

#[test]
fn a_changed_on_disk_during_apply_ends_the_run_and_the_next_run_succeeds() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    let web = conn(&a, "web");
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    rename(&b, web, "web-renamed");
    now(&b);

    // a file-sync tool replaces connections.json while A's run is between read and apply
    let other = TempDir::new().unwrap();
    let other_st = AppState::open(Some(other.path().to_path_buf()));
    commands::create_folder(&other_st, None, None, "foreign".into()).unwrap();
    let foreign = fs::read(other.path().join(fernjo_core::FILE_NAME)).unwrap();
    let path = a.data.join(fernjo_core::FILE_NAME);
    let port = HookPort::new(&a.st, || fs::write(&path, &foreign).unwrap());
    assert!(run_background_with(&a.st, &port));
    assert_eq!(
        port.applies.get(),
        1,
        "the run ended, it did not start over"
    );
    let s = sync_status(&a.st).unwrap();
    assert_eq!(s.state, "error");
    assert_eq!(s.error.as_ref().unwrap().code, "changed_on_disk");
    assert_eq!(
        fs::read(&path).unwrap(),
        foreign,
        "the foreign file was not overwritten"
    );
    assert_eq!(names(&a), ["web"], "memory still holds the old tree");

    // the user reloads (what the notice bar's button does); the next run goes through
    commands::reload_tree(&a.st).unwrap();
    let s = now(&a);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(s.error, None);
    // the foreign tree and B's rename met: the rename beat the deletion of "web"
    assert_eq!(names(&a), ["foreign", "web-renamed"]);
}

#[test]
fn locked_vault_with_a_head_that_changed_only_the_connections_still_syncs() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    setup(&a, &hub, "a");
    // B has no vault yet: the join installs the group's file, locked
    join(&b, &a, "b");
    assert_eq!(vault_state(&b), "locked");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    vault_lock(&b.st).unwrap();

    // only the connections changed on A
    folder(&a, "from-a");
    now(&a);
    let s = now(&b);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(names(&b), ["from-a"]);
    assert_eq!(vault_state(&b), "locked");
    assert_eq!(vault_file(&b), vault_file(&a));

    // only the vault file changed on A
    credential(&a, "two");
    now(&a);
    let s = now(&b);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(vault_state(&b), "locked", "adopted without an unlock");
    assert_eq!(vault_file(&b), vault_file(&a));
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    assert_eq!(credential_names(&b), ["one", "two"]);
}

#[test]
fn an_unlocked_vault_stays_unlocked_when_the_file_is_adopted() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    credential(&a, "from-a");
    now(&a);
    assert_eq!(now(&b).state, "ok");
    assert_eq!(vault_state(&b), "unlocked");
    assert_eq!(credential_names(&b), ["from-a"]);
}

#[test]
fn both_sides_changing_a_locked_vault_waits_for_the_unlock_and_then_merges() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    credential(&a, "from-a");
    credential(&b, "from-b");
    now(&a);
    vault_lock(&b.st).unwrap();
    let s = now(&b);
    assert_eq!(s.state, "waiting_for_unlock");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    let s = now(&b);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(credential_names(&b), ["from-a", "from-b"]);
    assert_eq!(vault_state(&b), "unlocked");
}

#[test]
fn an_idle_lock_between_read_and_apply_ends_the_run_untouched_and_the_next_run_succeeds() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    // both sides changed the vault, so the merge needs the key at apply time
    credential(&a, "from-a");
    credential(&b, "from-b");
    now(&a);

    // the idle lock fires while B's run is between read (unlocked) and apply; locking does not
    // change the vault file bytes, so the engine's "did anything change" check cannot see it
    let before = vault_file(&b);
    let tree_before = names(&b);
    let port = HookPort::new(&b.st, || {
        vault_lock(&b.st).unwrap();
    });
    assert!(run_background_with(&b.st, &port));
    assert_eq!(port.applies.get(), 1);
    let s = sync_status(&b.st).unwrap();
    assert_eq!(s.state, "error");
    assert_eq!(s.error.as_ref().unwrap().code, "vault_locked");
    // rolled back: no header, generation floor or entry changed on disk, the tree is as it was
    assert_eq!(vault_file(&b), before);
    assert_eq!(names(&b), tree_before);
    assert_eq!(vault_state(&b), "locked");

    // after the unlock the next run merges both credentials
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    let s = now(&b);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(credential_names(&b), ["from-a", "from-b"]);
}

#[test]
fn findings_stay_until_dismissed_and_are_not_repeated() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    let web = conn(&a, "web");
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    rename(&b, web, "b-name");
    now(&b);
    rename(&a, web, "a-name");
    let s = now(&a);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(s.findings.len(), 1);
    assert_eq!(s.findings[0].kind, "conflictCopy");
    assert!(s.findings[0].message.contains("changed on two devices"));
    assert!(!s.findings[0].id.is_empty());
    // the next runs show it again as long as it is not dismissed, but never make a second copy
    let s = now(&a);
    assert_eq!(s.findings.len(), 1);
    assert_eq!(
        names(&a).iter().filter(|n| n.contains("conflict")).count(),
        1
    );
    sync_dismiss_findings(&a.st).unwrap();
    assert!(sync_status(&a.st).unwrap().findings.is_empty());
}

#[test]
fn findings_of_an_offline_run_are_kept_and_deduplicated() {
    let d = dev();
    let f = Finding::KeptEdit {
        node: fernjo_core::NodeId::nil(),
        name: "web".into(),
    };
    record(
        &d.st,
        Ok(Outcome::Offline {
            pending_changes: 1,
            findings: vec![f.clone()],
        }),
    );
    record(
        &d.st,
        Ok(Outcome::Synced {
            findings: vec![f.clone()],
            seq: 3,
        }),
    );
    let rt = runtime(&d.st);
    assert_eq!(rt.findings, vec![f]);
    assert_eq!(rt.last, LastOutcome::Ok);
}

#[test]
fn a_hub_secret_that_stopped_working_is_replaced_after_the_hub_accepts_the_new_one() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "a");
    let group = state_file(&a).group;
    let (_, key) = a.st.sync_secrets.get(group).unwrap().unwrap();
    // the stored secret is stale
    a.st.sync_secrets
        .set(group, &sec(WRONG_SECRET), &key)
        .unwrap();
    let s = now(&a);
    assert_eq!(s.state, "error");
    assert_eq!(s.error.as_ref().unwrap().code, "hub_unauthorized");

    // a wrong new secret stores nothing
    let e = sync_edit_secret(&a.st, sec("SENTINEL-nope-5")).unwrap_err();
    assert_eq!(e.code, "hub_unauthorized");
    assert!(!shown(&e).contains("SENTINEL-nope-5"));
    let (stored, _) = a.st.sync_secrets.get(group).unwrap().unwrap();
    assert_eq!(stored.expose_secret(), WRONG_SECRET);

    let s = sync_edit_secret(&a.st, sec(HUB_SECRET)).unwrap();
    assert_eq!(s.error, None);
    let (stored, key_after) = a.st.sync_secrets.get(group).unwrap().unwrap();
    assert_eq!(stored.expose_secret(), HUB_SECRET);
    assert_eq!(key_after.as_bytes(), key.as_bytes(), "the key is unchanged");
    assert_eq!(now(&a).state, "ok");
}

#[test]
fn a_rolled_back_hub_is_reported_and_reseed_repairs_it() {
    let hub = start_hub();
    let a = dev();
    conn(&a, "web");
    setup(&a, &hub, "a");
    // this device has seen a newer state than the hub holds now
    let mut st = state_file(&a);
    st.last_seq = 5;
    StateStore::new(a.device.clone()).save(&st).unwrap();
    let s = now(&a);
    assert_eq!(s.state, "rolled_back");
    assert_eq!(s.error.as_ref().unwrap().code, "hub_rolled_back");
    let s = sync_reseed(&a.st).unwrap();
    assert_eq!(s.state, "ok");
    assert_eq!(state_file(&a).last_seq, 6);
    assert_eq!(now(&a).state, "ok");
}

/// Final M11 review I2: a head whose vault is older than this device's base (another device
/// re-seeded the hub with an older vault) is refused (V5) and must not leave a dead end: the status
/// is the rolled-back one with its Re-seed action, and Re-seed pushes this device's newer vault.
#[test]
fn an_older_vault_in_the_head_offers_reseed_and_reseed_keeps_the_newer_local_vault() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    // A saves the vault twice more and syncs once in between; B never syncs again
    credential(&a, "two");
    assert_eq!(now(&a).state, "ok");
    credential(&a, "three");
    // B re-seeds the hub with its older vault, above A's last seq
    assert_eq!(sync_reseed(&b.st).unwrap().state, "ok");

    let s = now(&a);
    assert_eq!(s.state, "rolled_back", "{:?}", s.error);
    let error = s.error.as_ref().unwrap();
    assert_eq!(error.code, "vault_rolled_back");
    assert!(!error.message.contains(MASTER));
    // the refusal stays until the hub is repaired, and the local vault is untouched
    assert_eq!(now(&a).state, "rolled_back");
    assert_eq!(credential_names(&a), ["one", "three", "two"]);

    let s = sync_reseed(&a.st).unwrap();
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(s.error, None);
    assert_eq!(credential_names(&a), ["one", "three", "two"]);
    assert_eq!(now(&a).state, "ok");
    // the other device follows the re-seeded, newer vault
    assert_eq!(now(&b).state, "ok");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    assert_eq!(credential_names(&b), ["one", "three", "two"]);
}

/// Re-review N2: the vault refuses a head below this device's mark, but here the local file is
/// itself below the mark (an "open the older version" whose re-stamp save failed) and the head is
/// newer than the local file. Re-seed would push the older local vault over it, so the status
/// is a plain error with its own code and no Re-seed offer; a save of the vault on this device
/// raises the local generation above the mark, and the next run merges the head.
#[test]
fn a_head_newer_than_a_local_vault_below_the_mark_is_not_offered_reseed() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    // A's handle ends up as after a failed re-stamp: unlocked, its file (still the base) below
    // the device mark. Raising the mark and attaching the store again is that state without
    // having to make a save fail.
    {
        let mut slot = a.st.vault.lock().unwrap();
        let VaultState::Open(v) = &mut *slot else {
            panic!("A has a vault");
        };
        let generation = v.generation().unwrap();
        a.st.generations.raise(v.id(), generation + 10);
        v.attach_generation_store(Arc::clone(&a.st.generations));
    }
    // B saves the vault once (a generation between A's file and A's mark) and pushes it
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    credential(&b, "two");
    assert_eq!(now(&b).state, "ok");
    let before = vault_file(&a);

    let s = now(&a);
    assert_eq!(s.state, "error", "{:?}", s.error);
    assert_eq!(s.error.as_ref().unwrap().code, "vault_below_mark");
    assert_ne!(s.state, "rolled_back", "no Re-seed offer");
    assert_eq!(vault_file(&a), before, "nothing was adopted");
    assert_eq!(credential_names(&a), ["one"]);

    // any save on this device lifts the local file above the mark; the next run merges
    credential(&a, "three");
    let s = now(&a);
    assert_eq!(s.state, "ok", "{:?}", s.error);
    assert_eq!(credential_names(&a), ["one", "three", "two"]);
}

fn head_seq(hub: &Hub) -> Option<u64> {
    HubClient::new(&HubConfig {
        url: hub.url(),
        secret: sec(HUB_SECRET),
        ca_pem: None,
    })
    .unwrap()
    .get_head()
    .unwrap()
    .map(|(seq, _)| seq)
}

#[test]
fn a_vault_file_planted_behind_the_vault_is_never_pushed() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    let older = vault_file(&a);
    setup(&a, &hub, "a");
    join(&b, &a, "b");
    credential(&a, "two");
    assert_eq!(now(&a).state, "ok");
    assert_eq!(head_seq(&hub), Some(2));

    // a writer of the data folder puts the older copy back while fernjo runs
    let path = a.data.join(fernjo_vault::FILE_NAME);
    fs::write(&path, &older).unwrap();
    let s = now(&a);
    assert_eq!(s.state, "error");
    assert_eq!(s.error.as_ref().unwrap().code, "changed_on_disk");
    assert_eq!(codes(sync_reseed(&a.st)), "changed_on_disk");
    assert_eq!(head_seq(&hub), Some(2), "nothing was pushed");
    assert_eq!(vault_file(&a), older, "the planted file is left alone");
    // the other device still gets the real vault
    assert_eq!(now(&b).state, "ok");
    vault_unlock(&b.st, sec(MASTER), false).unwrap();
    assert_eq!(credential_names(&b), ["one", "two"]);

    // set-up refuses a planted file too and uploads nothing
    let (c, empty_hub) = (dev(), start_hub());
    vault_create(&c.st, sec(MASTER), false).unwrap();
    let first = vault_file(&c);
    credential(&c, "x");
    fs::write(c.data.join(fernjo_vault::FILE_NAME), &first).unwrap();
    let r = sync_setup(
        &c.st,
        empty_hub.url(),
        sec(HUB_SECRET),
        None,
        "c".to_owned(),
    );
    assert_eq!(codes(r), "changed_on_disk");
    assert_eq!(head_seq(&empty_hub), None, "nothing was uploaded");
    assert_eq!(sync_status(&c.st).unwrap().state, "off");
}

#[test]
fn leave_removes_state_base_and_keychain_items_and_keeps_local_data() {
    let hub = start_hub();
    let a = dev();
    conn(&a, "web");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    setup(&a, &hub, "a");
    let group = state_file(&a).group;
    fs::write(a.device.join("known_hosts.confirmed"), b"keep").unwrap();
    let (tree_before, vault_before) = (tree(&a), vault_file(&a));
    assert!(a.st.sync_secrets.get(group).unwrap().is_some());

    let s = sync_leave(&a.st).unwrap();
    assert_eq!(s.state, "off");
    assert!(!a.device.join("sync.json").exists());
    assert!(!a.device.join("sync-base.json").exists());
    assert!(a.st.sync_secrets.get(group).unwrap().is_none());
    assert_eq!(
        fs::read(a.device.join("known_hosts.confirmed")).unwrap(),
        b"keep"
    );
    assert_eq!(tree(&a), tree_before);
    assert_eq!(vault_file(&a), vault_before);
    assert_eq!(codes(sync_leave(&a.st)), "not_paired");
    assert_eq!(sync_status(&a.st).unwrap().state, "off");
    // and the device can set up again against an empty hub
    let hub2 = start_hub();
    assert_eq!(setup(&a, &hub2, "a again").state, "ok");
}

/// A secrets store that works except that it refuses to delete.
struct StuckDelete(fernjo_sync::secrets::MemorySyncSecrets);

impl fernjo_sync::secrets::SyncSecrets for StuckDelete {
    fn get(
        &self,
        group: fernjo_sync::GroupId,
    ) -> Result<Option<(SecretString, fernjo_sync::SyncKey)>, SyncError> {
        self.0.get(group)
    }
    fn set(
        &self,
        group: fernjo_sync::GroupId,
        hub_secret: &SecretString,
        key: &fernjo_sync::SyncKey,
    ) -> Result<(), SyncError> {
        self.0.set(group, hub_secret, key)
    }
    fn delete(&self, _: fernjo_sync::GroupId) -> Result<(), SyncError> {
        Err(SyncError::KeychainUnavailable(
            "the keychain is locked".to_owned(),
        ))
    }
}

#[test]
fn leave_completes_when_the_keychain_cannot_delete_and_names_the_leftover_item() {
    let hub = start_hub();
    let mut a = dev();
    conn(&a, "web");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    a.st.sync_secrets = Box::new(StuckDelete(Default::default()));
    setup(&a, &hub, "a");
    let group = state_file(&a).group;
    let (tree_before, vault_before) = (tree(&a), vault_file(&a));

    let s = sync_leave(&a.st).unwrap();
    assert_eq!(s.state, "off", "the device left sync");
    assert!(!a.device.join("sync.json").exists());
    assert!(!a.device.join("sync-base.json").exists());
    assert_eq!(tree(&a), tree_before, "local data stays");
    assert_eq!(vault_file(&a), vault_before);
    let e = s.error.expect("the leftover item is reported");
    assert_eq!(e.code, "keychain_unavailable");
    assert!(
        e.message.contains(&format!("sync-{group}")),
        "{}",
        e.message
    );
    assert!(e.message.contains("fernjo"), "{}", e.message);
    // the answer carries no secret
    assert!(!shown(&e).contains(HUB_SECRET));
    // later status reads are plain "off" and the device can leave no more
    let later = sync_status(&a.st).unwrap();
    assert_eq!(later.state, "off");
    assert_eq!(later.error, None);
    assert_eq!(codes(sync_leave(&a.st)), "not_paired");
}

#[test]
fn join_edit_secret_reseed_leave_and_the_event_payload_carry_no_secret() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    vault_create(&a.st, sec(MASTER), false).unwrap();
    credential(&a, "one");
    setup(&a, &hub, "device-a");
    let code = sync_pairing_code(&a.st).unwrap();
    let group = state_file(&a).group;
    let (_, key) = a.st.sync_secrets.get(group).unwrap().unwrap();
    let raw = *key.as_bytes();
    let forbidden = [
        HUB_SECRET.to_owned(),
        WRONG_SECRET.to_owned(),
        MASTER.to_owned(),
        "pw-1".to_owned(),
        code.clone(),
        base64::engine::general_purpose::STANDARD.encode(raw),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw),
        raw.iter().map(|b| format!("{b:02x}")).collect::<String>(),
    ];
    let mut seen: Vec<(String, String)> = Vec::new();
    let mut note = |what: &str, text: String| seen.push((what.to_owned(), text));
    // the `sync-status` event carries `build_status`, serialised as the runner emits it
    let event = |d: &Dev| serde_json::to_string(&build_status(&d.st)).unwrap();

    // join: the confirmation step, a refused code, the real join and the status after it
    let asked = sync_join(&b.st, &sec(&code), "device-b".into(), false).unwrap();
    assert_eq!(asked.state, "needs_confirmation");
    note("join (ask)", shown(&asked));
    let e = sync_join(
        &b.st,
        &sec("fernjo-sync-1:garbage"),
        "device-b".into(),
        true,
    )
    .unwrap_err();
    note("join (bad code)", shown(&e));
    let joined = sync_join(&b.st, &sec(&code), "device-b".into(), true).unwrap();
    assert_eq!(joined.state, "ok");
    note("join", shown(&joined));
    note("event after join", event(&b));
    let e = sync_join(&b.st, &sec(&code), "device-b".into(), true).unwrap_err();
    assert_eq!(e.code, "already_paired");
    note("join again", shown(&e));

    // edit secret: refused with a wrong one, accepted with the right one
    let e = sync_edit_secret(&a.st, sec(WRONG_SECRET)).unwrap_err();
    assert_eq!(e.code, "hub_unauthorized");
    note("edit secret (wrong)", shown(&e));
    let s = sync_edit_secret(&a.st, sec(HUB_SECRET)).unwrap();
    note("edit secret", shown(&s));
    note("event after edit secret", event(&a));

    // a run that ends in an error (the stored secret went stale), as status and as event
    a.st.sync_secrets
        .set(group, &sec(WRONG_SECRET), &key)
        .unwrap();
    let s = now(&a);
    assert_eq!(s.state, "error");
    note("now (error)", shown(&s));
    note("event (error)", event(&a));
    sync_edit_secret(&a.st, sec(HUB_SECRET)).unwrap();

    // a conflict notice goes out with its text
    folder(&a, "x");
    now(&a);
    let s = now(&b);
    note("now (applied)", shown(&s));
    note("event (applied)", event(&b));

    // reseed and leave
    let s = sync_reseed(&a.st).unwrap();
    note("reseed", shown(&s));
    note("event after reseed", event(&a));
    let s = sync_leave(&b.st).unwrap();
    note("leave", shown(&s));
    note("event after leave", event(&b));
    note("AppState Debug", format!("{:?}", b.st));
    note("runtime Debug", format!("{:?}", a.st.sync));

    // a leave that could not clear the keychain names the item, not its contents
    let mut c = dev();
    c.st.sync_secrets = Box::new(StuckDelete(Default::default()));
    let hub2 = start_hub();
    setup(&c, &hub2, "device-c");
    let s = sync_leave(&c.st).unwrap();
    note("leave (keychain stuck)", shown(&s));
    note("event after stuck leave", event(&c));

    assert!(seen.len() > 15, "every step was recorded");
    for (what, text) in &seen {
        for s in &forbidden {
            assert!(!text.contains(s), "{what} leaked {s}: {text}");
        }
    }
}

#[test]
fn a_password_notice_reaches_the_ui_as_a_finding_with_the_vault_id_and_no_secret() {
    let vault = uuid::Uuid::from_u128(0xAB);
    let view = FindingView::of(&Finding::PasswordChangeDropped { vault });
    assert_eq!(view.kind, "passwordChangeDropped");
    assert_eq!(view.id, vault.to_string());
    assert_eq!(view.name, "Vault");
    assert!(view.message.contains("master password"), "{}", view.message);
    assert!(view.message.contains("Change it again"), "{}", view.message);
    let wire = serde_json::to_value(&view).unwrap();
    assert_eq!(wire["kind"], "passwordChangeDropped");
}

#[test]
fn leave_works_with_an_unusable_state_file() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "a");
    fs::write(a.device.join("sync.json"), b"{ not json").unwrap();
    let s = sync_status(&a.st).unwrap();
    assert_eq!(s.state, "error");
    assert_eq!(s.error.as_ref().unwrap().code, "state_unusable");
    let s = sync_leave(&a.st).unwrap();
    assert_eq!(s.state, "off");
    assert!(!a.device.join("sync.json").exists());
}

#[test]
fn runs_never_overlap_and_sync_now_reports_the_busy_state() {
    let hub = start_hub();
    let a = dev();
    setup(&a, &hub, "a");
    let guard = try_begin(&a.st).expect("the slot is free");
    assert!(try_begin(&a.st).is_none());
    assert!(
        !run_background(&a.st),
        "a trigger that finds the slot taken is dropped"
    );
    let s = sync_now(&a.st).unwrap();
    assert!(s.running);
    assert_eq!(codes(sync_leave(&a.st)), "sync_busy");
    assert_eq!(codes(sync_reseed(&a.st)), "sync_busy");
    drop(guard);
    assert!(!sync_status(&a.st).unwrap().running);
    assert!(run_background(&a.st));
    assert!(
        !sync_status(&a.st).unwrap().running,
        "the slot is released after a run"
    );
}

#[test]
fn a_vault_file_that_cannot_be_read_ends_the_run_and_is_never_replaced() {
    let hub = start_hub();
    let a = dev();
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");
    fs::write(a.data.join(fernjo_vault::FILE_NAME), b"garbage").unwrap();
    let s = now(&a);
    assert_eq!(s.state, "error");
    // the file is not the one the vault handle knows (since M11 review #30)
    assert_eq!(s.error.as_ref().unwrap().code, "changed_on_disk");
    assert_eq!(
        fs::read(a.data.join(fernjo_vault::FILE_NAME)).unwrap(),
        b"garbage"
    );
    let v: Value = serde_json::to_value(&s).unwrap();
    assert_eq!(v["state"], "error");
    assert!(v["pendingChanges"].is_number());
}

// ---- apply: consistency of the vault and tree writes -------------------------------------------

/// A device with a tree and an unlocked vault, and the bytes of a vault of another id.
fn device_with_vault() -> (Dev, Vec<u8>, Dev) {
    let x = dev();
    folder(&x, "mine");
    vault_create(&x.st, sec(MASTER), false).unwrap();
    credential(&x, "mine-entry");
    let other = dev();
    vault_create(&other.st, sec("SENTINEL-other-master-88"), false).unwrap();
    let bytes = vault_file(&other);
    (x, bytes, other)
}

/// A `connections.json` written by somebody else.
fn write_foreign_tree(d: &Dev) -> Vec<u8> {
    let other = TempDir::new().unwrap();
    let other_st = AppState::open(Some(other.path().to_path_buf()));
    commands::create_folder(&other_st, None, None, "foreign".into()).unwrap();
    let foreign = fs::read(other.path().join(fernjo_core::FILE_NAME)).unwrap();
    fs::write(d.data.join(fernjo_core::FILE_NAME), &foreign).unwrap();
    foreign
}

#[test]
fn a_foreign_connections_file_refuses_an_apply_with_a_vault_change_before_any_vault_write() {
    for merged in [false, true] {
        let (x, other_bytes, _other) = device_with_vault();
        let port = AppPort(&x.st);
        let expected = port.read().unwrap();
        let (vault_before, id_before) = (vault_file(&x), vault_status(&x.st).unwrap().path);
        let foreign = write_foreign_tree(&x);
        let vault = if merged {
            VaultChange::Merged {
                head: vault_before.clone(),
                base_generation: 0,
                entries: Vec::new(),
            }
        } else {
            VaultChange::Adopt(other_bytes)
        };
        let err = port
            .apply(
                &expected,
                LocalChange {
                    tree: Some(Tree::new()),
                    vault,
                },
            )
            .unwrap_err();
        assert_eq!(err, SyncError::LocalFailed("changed_on_disk".into()));
        assert_eq!(vault_file(&x), vault_before, "merged: {merged}");
        assert_eq!(vault_state(&x), "unlocked", "the handle is untouched");
        assert_eq!(vault_status(&x.st).unwrap().path, id_before);
        assert_eq!(credential_names(&x), ["mine-entry"]);
        assert_eq!(x.st.sync_applied.load(Ordering::SeqCst), 0);
        assert_eq!(
            fs::read(x.data.join(fernjo_core::FILE_NAME)).unwrap(),
            foreign
        );
    }
}

#[test]
fn a_failed_tree_write_after_a_vault_write_still_counts_as_applied() {
    let (x, other_bytes, other) = device_with_vault();
    let port = AppPort(&x.st);
    let expected = port.read().unwrap();
    // a tree deeper than the limit passes the fingerprint check and fails in `replace_tree`
    let mut deep = Node {
        id: fernjo_core::NodeId::new_v4(),
        name: "leaf".into(),
        description: String::new(),
        settings: Default::default(),
        kind: NodeKind::Folder {
            children: Vec::new(),
        },
    };
    for _ in 0..=fernjo_core::MAX_TREE_DEPTH {
        deep = Node {
            id: fernjo_core::NodeId::new_v4(),
            name: "d".into(),
            description: String::new(),
            settings: Default::default(),
            kind: NodeKind::Folder {
                children: vec![deep],
            },
        };
    }
    let tree_before = tree(&x);
    let err = port
        .apply(
            &expected,
            LocalChange {
                tree: Some(Tree::from_children(vec![deep])),
                vault: VaultChange::Adopt(other_bytes.clone()),
            },
        )
        .unwrap_err();
    assert_eq!(err, SyncError::LocalFailed("too_deep".into()));
    // the vault file was replaced, so the UI is told to look again
    assert_eq!(vault_file(&x), other_bytes);
    assert_eq!(vault_state(&x), "locked");
    assert_eq!(x.st.sync_applied.load(Ordering::SeqCst), 1);
    assert_eq!(tree(&x), tree_before, "the tree stayed");
    let _ = other;
}

#[test]
fn a_join_refused_by_the_fingerprint_leaves_the_device_unpaired_and_untouched() {
    let hub = start_hub();
    let (a, b) = (dev(), dev());
    folder(&a, "from-a");
    vault_create(&a.st, sec(MASTER), false).unwrap();
    setup(&a, &hub, "a");
    folder(&b, "mine");
    vault_create(&b.st, sec("SENTINEL-other-master-77"), false).unwrap();
    let vault_before = vault_file(&b);
    let foreign = write_foreign_tree(&b);
    let group = state_file(&a).group;

    let code = sec(&sync_pairing_code(&a.st).unwrap());
    let err = sync_join(&b.st, &code, "b".into(), true).unwrap_err();
    assert_eq!(err.code, "changed_on_disk");
    assert!(!b.device.join("sync.json").exists());
    assert!(!b.device.join("sync-base.json").exists());
    assert!(b.st.sync_secrets.get(group).unwrap().is_none());
    assert_eq!(vault_file(&b), vault_before);
    assert_eq!(vault_state(&b), "unlocked");
    assert_eq!(
        fs::read(b.data.join(fernjo_core::FILE_NAME)).unwrap(),
        foreign
    );
    assert_eq!(b.st.sync_applied.load(Ordering::SeqCst), 0);
    assert_eq!(sync_status(&b.st).unwrap().state, "off");
}

#[test]
fn apply_refuses_with_local_changed_when_only_the_vault_file_differs_from_expected() {
    let (x, other_bytes, _other) = device_with_vault();
    let port = AppPort(&x.st);
    let expected = port.read().unwrap();
    // the user saves a credential after the engine read the data: tree equal, vault file not
    credential(&x, "added-meanwhile");
    let vault_now = vault_file(&x);
    let err = port
        .apply(
            &expected,
            LocalChange {
                tree: None,
                vault: VaultChange::Adopt(other_bytes),
            },
        )
        .unwrap_err();
    assert_eq!(err, SyncError::LocalChanged);
    assert_eq!(vault_file(&x), vault_now);
    assert_eq!(x.st.sync_applied.load(Ordering::SeqCst), 0);
}

#[test]
fn reset_keeps_the_running_flag_of_the_command_that_holds_the_slot() {
    let d = dev();
    let _guard = try_begin(&d.st).unwrap();
    runtime(&d.st).reset();
    assert!(runtime(&d.st).running);
    assert!(try_begin(&d.st).is_none());
}
