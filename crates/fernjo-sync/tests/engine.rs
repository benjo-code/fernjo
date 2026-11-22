//! The sync engine against the real hub (`fernjo-hub`, in process, on loopback), with in-memory
//! local ports (a tree plus a real `fernjo_vault::Vault` in a temp folder) and one state folder
//! per device.

use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use fernjo_core::{Node, NodeId, NodeKind, NodePatch, Protocol, Settings, Tree};
use fernjo_hub::{HubStore, config, router, serve};
use fernjo_sync::client::{Expected, HubClient, HubConfig};
use fernjo_sync::engine::{Engine, Outcome};
use fernjo_sync::merge::{Finding, MergeContext};
use fernjo_sync::ports::{LocalChange, LocalPort, LocalState, VaultChange, VaultSide};
use fernjo_sync::secrets::{MemorySyncSecrets, SyncSecrets};
use fernjo_sync::state::{BASE_FILE, STATE_FILE, StateStore};
use fernjo_sync::{DeviceInfo, GroupId, Snapshot, SyncError, SyncKey, seal};
use fernjo_vault::{FILE_NAME, KdfParams, NewEntry, Secret, Vault, inspect_file, read_entries};
use secrecy::SecretString;
use tempfile::TempDir;
use uuid::Uuid;

const SECRET: &str = "test-secret";
const PW: &str = "correct horse battery";
const DATE: &str = "2026-11-22";

// ---- hub ---------------------------------------------------------------------------------

struct Hub {
    _dir: TempDir,
    addr: SocketAddr,
}

impl Hub {
    fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    fn client(&self) -> HubClient {
        HubClient::new(&cfg(&self.url())).unwrap()
    }
}

fn cfg(url: &str) -> HubConfig {
    HubConfig {
        url: url.to_owned(),
        secret: SecretString::from(SECRET.to_owned()),
        ca_pem: None,
    }
}

fn start() -> Hub {
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
                router(store, config::hash_secret(SECRET)),
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

/// A TCP proxy in front of the hub. While `fail_puts` is set it drops every `PUT` before the hub
/// sees it (a lost connection between "apply locally" and "push"). While `swallow_put_responses`
/// is set it forwards the `PUT` (the hub stores the head) but never relays the answer and closes
/// the connection (the process dies, or the network drops, right after the push).
struct Proxy {
    addr: SocketAddr,
    fail_puts: Arc<AtomicBool>,
    swallow_put_responses: Arc<AtomicBool>,
}

fn start_proxy(target: SocketAddr) -> Proxy {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let fail_puts = Arc::new(AtomicBool::new(false));
    let swallow_put_responses = Arc::new(AtomicBool::new(false));
    let (flag, swallow_flag) = (fail_puts.clone(), swallow_put_responses.clone());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(client) = stream else { continue };
            let Ok(upstream) = TcpStream::connect(target) else {
                continue;
            };
            let flag = flag.clone();
            let swallow_flag = swallow_flag.clone();
            // set for a connection whose request is a PUT that must not be answered
            let swallow = Arc::new(AtomicBool::new(false));
            let swallow_here = swallow.clone();
            let (mut c_read, mut u_write) =
                (client.try_clone().unwrap(), upstream.try_clone().unwrap());
            let (mut u_read, mut c_write) =
                (upstream.try_clone().unwrap(), client.try_clone().unwrap());
            thread::spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = match c_read.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if flag.load(Ordering::SeqCst) && buf[..n].starts_with(b"PUT ") {
                        let _ = client.shutdown(std::net::Shutdown::Both);
                        let _ = upstream.shutdown(std::net::Shutdown::Both);
                        break;
                    }
                    if swallow_flag.load(Ordering::SeqCst) && buf[..n].starts_with(b"PUT ") {
                        swallow_here.store(true, Ordering::SeqCst);
                    }
                    if u_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            });
            thread::spawn(move || {
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let n = match u_read.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if swallow.load(Ordering::SeqCst) {
                        break;
                    }
                    if c_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                let _ = c_write.shutdown(std::net::Shutdown::Both);
            });
        }
    });
    Proxy {
        addr,
        fail_puts,
        swallow_put_responses,
    }
}

// ---- local port -------------------------------------------------------------------------

type Hook = Box<dyn FnOnce(&Port)>;

struct Port {
    dir: TempDir,
    tree: Mutex<Tree>,
    vault: Mutex<Option<Vault>>,
    fake_vault: Mutex<Option<Vec<u8>>>,
    hook: RefCell<Option<Hook>>,
    adopts: AtomicUsize,
}

impl Port {
    fn new() -> Port {
        Port {
            dir: TempDir::new().unwrap(),
            tree: Mutex::new(Tree::new()),
            vault: Mutex::new(None),
            fake_vault: Mutex::new(None),
            hook: RefCell::new(None),
            adopts: AtomicUsize::new(0),
        }
    }

    fn vault_path(&self) -> PathBuf {
        self.dir.path().join(FILE_NAME)
    }

    fn tree(&self) -> Tree {
        self.tree.lock().unwrap().clone()
    }

    fn add_connection(&self, name: &str) -> NodeId {
        self.tree
            .lock()
            .unwrap()
            .create_connection(None, None, name, Protocol::Ssh, "h.example")
            .unwrap()
    }

    fn rename(&self, id: NodeId, name: &str) {
        self.tree
            .lock()
            .unwrap()
            .update(
                id,
                NodePatch {
                    name: Some(name.to_owned()),
                    ..NodePatch::default()
                },
            )
            .unwrap();
    }

    fn create_vault(&self) {
        let vault = Vault::create_with(
            &self.vault_path(),
            &SecretString::from(PW.to_owned()),
            KdfParams::TEST,
        )
        .unwrap();
        *self.vault.lock().unwrap() = Some(vault);
    }

    fn add_entry(&self, name: &str) {
        self.vault
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .add(NewEntry {
                name: name.to_owned(),
                username: None,
                domain: None,
                secret: Secret::Password {
                    password: SecretString::from("pw".to_owned()),
                },
                notes: String::new(),
            })
            .unwrap();
    }

    fn unlock(&self) {
        self.vault
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .unlock(&SecretString::from(PW.to_owned()))
            .unwrap();
    }

    fn change_password(&self, new: &str) {
        self.vault
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .change_password(
                &SecretString::from(PW.to_owned()),
                &SecretString::from(new.to_owned()),
            )
            .unwrap();
    }

    /// Whether the vault file on disk opens with `password` (a fresh handle, so the state of
    /// the one the engine uses does not matter).
    fn file_opens_with(&self, password: &str) -> bool {
        let mut vault = Vault::open(&self.vault_path()).unwrap().unwrap();
        vault
            .unlock(&SecretString::from(password.to_owned()))
            .is_ok()
    }

    fn lock(&self) {
        self.vault.lock().unwrap().as_mut().unwrap().lock();
    }

    fn vault_file(&self) -> Option<Vec<u8>> {
        self.vault
            .lock()
            .unwrap()
            .as_ref()
            .map(|v| v.file_bytes().unwrap())
    }

    fn entry_names(&self) -> Vec<String> {
        let guard = self.vault.lock().unwrap();
        let mut names: Vec<String> = guard
            .as_ref()
            .unwrap()
            .list()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        names.sort();
        names
    }

    fn generation(&self) -> u64 {
        let guard = self.vault.lock().unwrap();
        let vault = guard.as_ref().unwrap();
        let key = vault.data_key().unwrap();
        read_entries(&vault.file_bytes().unwrap(), &key)
            .unwrap()
            .generation
    }

    /// Puts `bytes` in place as the vault file without telling the engine (a copy by hand).
    fn force_vault_file(&self, bytes: &[u8]) {
        *self.vault.lock().unwrap() = None;
        std::fs::write(self.vault_path(), bytes).unwrap();
        *self.vault.lock().unwrap() = Vault::open(&self.vault_path()).unwrap();
    }

    fn snapshot_of_data(&self) -> (Tree, Option<Vec<u8>>) {
        (self.tree(), self.vault_file())
    }

    fn set_hook(&self, hook: impl FnOnce(&Port) + 'static) {
        *self.hook.borrow_mut() = Some(Box::new(hook));
    }
}

fn local_failed(code: &str) -> SyncError {
    SyncError::LocalFailed(code.to_owned())
}

impl LocalPort for Port {
    fn read(&self) -> Result<LocalState, SyncError> {
        let tree = self.tree();
        let guard = self.vault.lock().unwrap();
        let vault = match guard.as_ref() {
            Some(v) => {
                let file = v.file_bytes().map_err(|e| local_failed(e.code()))?;
                let unlocked = v.is_unlocked();
                Some(VaultSide {
                    file,
                    entries: if unlocked {
                        Some(v.export_entries().map_err(|e| local_failed(e.code()))?)
                    } else {
                        None
                    },
                    key: if unlocked {
                        Some(v.data_key().map_err(|e| local_failed(e.code()))?)
                    } else {
                        None
                    },
                })
            }
            None => self
                .fake_vault
                .lock()
                .unwrap()
                .clone()
                .map(|file| VaultSide {
                    file,
                    entries: None,
                    key: None,
                }),
        };
        Ok(LocalState { tree, vault })
    }

    fn apply(&self, expected: &LocalState, next: LocalChange) -> Result<(), SyncError> {
        let hook = self.hook.borrow_mut().take();
        if let Some(hook) = hook {
            hook(self);
        }
        if self.read()? != *expected {
            return Err(SyncError::LocalChanged);
        }
        if let Some(tree) = next.tree {
            *self.tree.lock().unwrap() = tree;
        }
        let mut guard = self.vault.lock().unwrap();
        match next.vault {
            VaultChange::Keep => {}
            VaultChange::Adopt(bytes) => {
                self.adopts.fetch_add(1, Ordering::SeqCst);
                let same_vault = match (guard.as_ref(), inspect_file(&bytes)) {
                    (Some(v), Ok(info)) => v.id() == info.id,
                    _ => false,
                };
                if same_vault {
                    guard
                        .as_mut()
                        .unwrap()
                        .adopt_file(&bytes)
                        .map_err(|e| local_failed(e.code()))?;
                } else {
                    *guard = None;
                    std::fs::write(self.vault_path(), &bytes).unwrap();
                    *guard = Vault::open(&self.vault_path()).unwrap();
                }
            }
            VaultChange::Merged { head, entries } => {
                guard
                    .as_mut()
                    .ok_or_else(|| local_failed("no_vault"))?
                    .adopt_merged(&head, entries)
                    .map_err(|e| local_failed(e.code()))?;
            }
        }
        Ok(())
    }

    fn vault_unlocked(&self) -> bool {
        self.vault
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(Vault::is_unlocked)
    }
}

// ---- devices ----------------------------------------------------------------------------

struct Device {
    name: String,
    dir: TempDir,
    state: StateStore,
    secrets: MemorySyncSecrets,
    port: Port,
    hub: HubClient,
}

impl Device {
    fn new(name: &str, hub_url: &str) -> Device {
        let dir = TempDir::new().unwrap();
        Device {
            name: name.to_owned(),
            state: StateStore::new(dir.path().join("device")),
            dir,
            secrets: MemorySyncSecrets::default(),
            port: Port::new(),
            hub: HubClient::new(&cfg(hub_url)).unwrap(),
        }
    }

    fn engine(&self) -> Engine<'_> {
        Engine {
            hub: &self.hub,
            state: &self.state,
            secrets: &self.secrets,
            local: &self.port,
            ctx: MergeContext {
                device_name: self.name.clone(),
                date: DATE.to_owned(),
            },
        }
    }

    fn run(&self) -> Outcome {
        self.engine().run_once().unwrap()
    }

    fn run_err(&self) -> SyncError {
        self.engine().run_once().unwrap_err()
    }

    fn state_files(&self) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
        let read = |name: &str| std::fs::read(self.dir.path().join("device").join(name)).ok();
        (read(STATE_FILE), read(BASE_FILE))
    }

    fn setup(&self, url: &str) -> fernjo_sync::pairing::PairingCode {
        self.engine().setup(&cfg(url), &self.name).unwrap()
    }

    fn join(&self, code: &fernjo_sync::pairing::PairingCode) {
        self.engine().join(code, &self.name).unwrap();
    }

    fn last_seq(&self) -> u64 {
        self.state.load().unwrap().unwrap().last_seq
    }

    /// Everything a refused run must leave alone.
    fn untouched(&self) -> Untouched {
        Untouched {
            data: self.port.snapshot_of_data(),
            files: self.state_files(),
        }
    }

    fn assert_untouched(&self, before: &Untouched) {
        assert_eq!(
            self.port.snapshot_of_data(),
            before.data,
            "local data changed"
        );
        assert_eq!(self.state_files(), before.files, "the state folder changed");
    }
}

struct Untouched {
    data: (Tree, Option<Vec<u8>>),
    files: (Option<Vec<u8>>, Option<Vec<u8>>),
}

fn pair(hub: &Hub) -> (Rc<Device>, Rc<Device>) {
    let a = Device::new("A", &hub.url());
    let code = a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.join(&code);
    (Rc::new(a), Rc::new(b))
}

/// Like `pair`, with a vault on device A (unlocked) that B adopts (and keeps locked).
fn pair_with_vault(hub: &Hub) -> (Rc<Device>, Rc<Device>) {
    let a = Device::new("A", &hub.url());
    a.port.create_vault();
    let code = a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.join(&code);
    (Rc::new(a), Rc::new(b))
}

fn names(tree: &Tree) -> Vec<String> {
    fn walk(nodes: &[Node], out: &mut Vec<String>) {
        for n in nodes {
            out.push(n.name.clone());
            if let NodeKind::Folder { children } = &n.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(tree.children(), &mut out);
    out.sort();
    out
}

fn connection(name: &str) -> Node {
    Node {
        id: Uuid::new_v4(),
        name: name.to_owned(),
        description: String::new(),
        settings: Settings::default(),
        kind: NodeKind::Connection {
            protocol: Protocol::Ssh,
            host: "h.example".to_owned(),
        },
    }
}

fn folder(children: Vec<Node>) -> Node {
    Node {
        id: Uuid::new_v4(),
        name: "f".to_owned(),
        description: String::new(),
        settings: Settings::default(),
        kind: NodeKind::Folder { children },
    }
}

fn nested(levels: usize) -> Tree {
    let mut node = connection("leaf");
    for _ in 1..levels {
        node = folder(vec![node]);
    }
    Tree::from_children(vec![node])
}

/// A head on `hub` sealed for `group` with `key` as envelope `seq`; the snapshot inside says
/// `inner_seq` and holds `tree`.
fn put_head(hub: &Hub, key: &SyncKey, group: GroupId, seq: u64, inner_seq: u64, tree: Tree) {
    let snapshot = Snapshot {
        format_version: 1,
        group_id: group,
        seq: inner_seq,
        writer: DeviceInfo {
            id: Uuid::from_u128(9),
            name: "other".to_owned(),
        },
        written_at: DATE.to_owned(),
        connections: tree,
        vault: None,
    };
    let bytes = seal(key, group, seq, &snapshot.to_plaintext().unwrap()).unwrap();
    let expected = match hub.client().get_head().unwrap() {
        Some((n, _)) => Expected::Seq(n),
        None => Expected::Empty,
    };
    hub.client().put_head(expected, seq, &bytes).unwrap();
}

fn key_of(dev: &Device, group: GroupId) -> SyncKey {
    let (_, key) = dev.secrets.get(group).unwrap().unwrap();
    key
}

fn group_of(dev: &Device) -> GroupId {
    dev.state.load().unwrap().unwrap().group
}

fn head_seq(hub: &Hub) -> Option<u64> {
    hub.client().get_head().unwrap().map(|(s, _)| s)
}

fn count_named(tree: &Tree, part: &str) -> usize {
    names(tree).iter().filter(|n| n.contains(part)).count()
}

// ---- setup and join -----------------------------------------------------------------------

#[test]
fn setup_uploads_seq_1_and_join_copies_it_to_the_second_device() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    a.port.add_connection("web");
    let code = a.setup(&hub.url());
    assert_eq!(head_seq(&hub), Some(1));
    assert_eq!(a.last_seq(), 1);
    assert_eq!(a.state.load_base().unwrap().unwrap().seq, 1);
    assert!(a.secrets.get(code.group).unwrap().is_some());
    assert_eq!(code.url, hub.url());

    let b = Device::new("B", &hub.url());
    b.port.add_connection("old local data");
    b.join(&code);
    assert_eq!(names(&b.port.tree()), ["web"]);
    assert_eq!(b.port.tree(), a.port.tree());
    assert_eq!(b.last_seq(), 1);
    let base = b.state.load_base().unwrap().unwrap();
    assert_eq!(base.seq, 1);
    assert_eq!(base.connections, a.port.tree());
    let (secret, key) = b.secrets.get(code.group).unwrap().unwrap();
    assert_eq!(secrecy::ExposeSecret::expose_secret(&secret), SECRET);
    assert_eq!(key.as_bytes(), code.key.as_bytes());
    assert_eq!(b.state.load().unwrap().unwrap().device.name, "B");
    assert_eq!(b.run_once_outcome(), "nothing");
}

impl Device {
    fn run_once_outcome(&self) -> &'static str {
        match self.run() {
            Outcome::NothingToDo => "nothing",
            Outcome::Synced { .. } => "synced",
            Outcome::Offline { .. } => "offline",
            Outcome::WaitingForUnlock => "waiting",
            Outcome::HubRolledBack { .. } => "rolled_back",
        }
    }
}

#[test]
fn setup_on_a_hub_with_a_head_is_hub_not_empty() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.port.add_connection("mine");
    let before = b.untouched();
    let err = b.engine().setup(&cfg(&hub.url()), "B").unwrap_err();
    assert_eq!(err.code(), "hub_not_empty");
    b.assert_untouched(&before);
    assert!(b.state.load().unwrap().is_none());
    assert_eq!(head_seq(&hub), Some(1));
}

#[test]
fn join_when_the_head_has_no_vault_keeps_the_local_vault_and_the_next_run_shares_it() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    let code = a.setup(&hub.url());

    let b = Device::new("B", &hub.url());
    b.port.create_vault();
    b.port.add_entry("b-entry");
    let vault_before = b.port.vault_file();
    b.join(&code);
    assert_eq!(b.port.vault_file(), vault_before, "the local vault stays");
    assert_eq!(b.port.adopts.load(Ordering::SeqCst), 0);

    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    assert_eq!(head_seq(&hub), Some(2));
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    assert_eq!(a.port.vault_file(), vault_before, "A adopted B's vault");
    a.port.unlock();
    assert_eq!(a.port.entry_names(), ["b-entry"]);
}

// ---- ordinary runs ---------------------------------------------------------------------------

#[test]
fn a_local_edit_reaches_the_other_device() {
    let hub = start();
    let (a, b) = pair(&hub);
    a.port.add_connection("new on A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    assert_eq!(b.port.tree(), a.port.tree());
    assert_eq!(names(&b.port.tree()), ["new on A"]);
    assert_eq!(b.last_seq(), 2);
    assert!(matches!(b.run(), Outcome::NothingToDo));
    assert!(matches!(a.run(), Outcome::NothingToDo));
    assert_eq!(head_seq(&hub), Some(2));
}

#[test]
fn a_vault_entry_added_on_one_device_reaches_the_other() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    assert!(!b.port.vault_unlocked());
    a.port.add_entry("shared secret");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    // B keeps the vault locked: only the head changed it (V1), so the file is adopted as it is
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    assert!(!b.port.vault_unlocked());
    assert_eq!(b.port.vault_file(), a.port.vault_file());
    b.port.unlock();
    assert_eq!(b.port.entry_names(), ["shared secret"]);
}

#[test]
fn both_sides_changed_the_vault_while_locked_waits_for_unlock_and_changes_nothing() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    b.port.unlock();
    a.port.add_entry("from A");
    a.port.add_connection("tree edit on A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    b.port.add_entry("from B");
    b.port.add_connection("tree edit on B");
    b.port.lock();

    let before = b.untouched();
    assert!(matches!(b.run(), Outcome::WaitingForUnlock));
    b.assert_untouched(&before);
    assert_eq!(head_seq(&hub), Some(2), "nothing was pushed");
    assert_eq!(
        b.port.adopts.load(Ordering::SeqCst),
        1,
        "only the join adopted"
    );

    b.port.unlock();
    assert!(matches!(b.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(b.port.entry_names(), ["from A", "from B"]);
    assert_eq!(names(&b.port.tree()), ["tree edit on A", "tree edit on B"]);
}

#[test]
fn both_sides_changed_the_vault_unlocked_merges_and_the_generation_is_above_both() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    b.port.unlock();
    a.port.add_entry("from A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    b.port.add_entry("from B");
    let (gen_a, gen_b) = (a.port.generation(), b.port.generation());

    assert!(matches!(b.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(b.port.entry_names(), ["from A", "from B"]);
    assert!(b.port.generation() > gen_a.max(gen_b));
    assert!(matches!(a.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(a.port.vault_file(), b.port.vault_file());
    assert_eq!(a.port.entry_names(), ["from A", "from B"]);
    assert!(matches!(b.run(), Outcome::NothingToDo));
}

const NEW_PW: &str = "a different password";

#[test]
fn a_password_change_dropped_by_a_both_changed_merge_is_reported_and_the_heads_password_stays() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    b.port.unlock();
    // B edits an entry and pushes first
    b.port.add_entry("from B");
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    // A changes the master password, then syncs: both sides changed the vault file (V3)
    a.port.change_password(NEW_PW);
    let Outcome::Synced { findings, seq: 3 } = a.run() else {
        panic!("expected a synced run");
    };
    let dropped: Vec<_> = findings
        .iter()
        .filter(|f| matches!(f, Finding::PasswordChangeDropped { .. }))
        .collect();
    assert_eq!(dropped.len(), 1, "findings: {findings:?}");
    let Finding::PasswordChangeDropped { vault } = dropped[0] else {
        unreachable!()
    };
    assert_eq!(
        *vault,
        inspect_file(&b.port.vault_file().unwrap()).unwrap().id
    );
    // the head's header won: the old password opens the merged vault, the new one does not
    assert!(a.port.file_opens_with(PW));
    assert!(!a.port.file_opens_with(NEW_PW));
    assert_eq!(a.port.entry_names(), ["from B"]);
    // no secret in the finding
    let debug = format!("{findings:?}");
    assert!(!debug.contains(PW) && !debug.contains(NEW_PW));
}

#[test]
fn a_password_change_that_only_the_head_made_gives_no_finding_and_is_taken() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    b.port.unlock();
    a.port.change_password(NEW_PW);
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    // B edited an entry (its wrap is still the base's), so both changed the vault
    b.port.add_entry("from B");
    let Outcome::Synced { findings, seq: 3 } = b.run() else {
        panic!("expected a synced run");
    };
    assert!(
        !findings
            .iter()
            .any(|f| matches!(f, Finding::PasswordChangeDropped { .. })),
        "findings: {findings:?}"
    );
    assert!(b.port.file_opens_with(NEW_PW), "the head's password stays");
    assert!(!b.port.file_opens_with(PW));
    assert_eq!(b.port.entry_names(), ["from B"]);
}

#[test]
fn a_head_vault_equal_to_the_local_file_is_not_adopted() {
    let hub = start();
    let (a, b) = pair_with_vault(&hub);
    a.port.add_entry("from A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    // B got the same file by hand (a copy), so base differs but local == head
    b.port.force_vault_file(&a.port.vault_file().unwrap());
    let adopts = b.port.adopts.load(Ordering::SeqCst);
    assert!(matches!(b.run(), Outcome::NothingToDo));
    assert_eq!(
        b.port.adopts.load(Ordering::SeqCst),
        adopts,
        "no adopt call"
    );
    assert_eq!(b.last_seq(), 2);
    assert_eq!(b.state.load_base().unwrap().unwrap().seq, 2);
}

#[test]
fn two_vaults_created_independently_are_not_merged() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    let code = a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.join(&code);
    b.port.create_vault();
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));

    a.port.create_vault();
    a.port.add_entry("A's own");
    let before = a.untouched();
    let err = a.run_err();
    assert_eq!(err.code(), "vault_conflict_different_vaults");
    a.assert_untouched(&before);
    assert_eq!(head_seq(&hub), Some(2));
}

// ---- conflicts and crashes -----------------------------------------------------------------

fn pair_with_server(hub: &Hub) -> (Rc<Device>, Rc<Device>, NodeId) {
    let a = Device::new("A", &hub.url());
    let id = a.port.add_connection("srv");
    let code = a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.join(&code);
    (Rc::new(a), Rc::new(b), id)
}

#[test]
fn a_conflict_makes_one_copy_and_a_second_run_changes_nothing() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    a.port.rename(id, "srv-a");
    b.port.rename(id, "srv-b");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));

    let Outcome::Synced { findings, seq } = b.run() else {
        panic!("expected a synced run");
    };
    assert_eq!(seq, 3);
    assert!(matches!(
        findings.as_slice(),
        [Finding::ConflictCopy { .. }]
    ));
    assert_eq!(count_named(&b.port.tree(), "conflict"), 1);
    assert_eq!(names(&b.port.tree()).len(), 2);
    assert!(b.port.tree().get(id).unwrap().name == "srv-a");

    assert!(matches!(b.run(), Outcome::NothingToDo));
    let Outcome::Synced { findings, .. } = a.run() else {
        panic!("A adopts the copy");
    };
    assert!(findings.is_empty());
    assert!(matches!(a.run(), Outcome::NothingToDo));
    assert_eq!(a.port.tree(), b.port.tree());
    assert_eq!(count_named(&a.port.tree(), "conflict"), 1);
    assert_eq!(head_seq(&hub), Some(3));
}

#[test]
fn a_crash_between_apply_and_push_is_repaired_by_the_next_run_without_a_duplicate() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    let proxy = start_proxy(hub.addr);
    let b_hub = HubClient::new(&cfg(&format!("http://{}", proxy.addr))).unwrap();
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = b_hub;

    a.port.rename(id, "srv-a");
    b.port.rename(id, "srv-b");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));

    // B merges and applies, but its push never arrives
    proxy.fail_puts.store(true, Ordering::SeqCst);
    // the run that applied the copy reports it, even though its push never arrived
    let Outcome::Offline { findings, .. } = b.run() else {
        panic!("expected offline");
    };
    assert!(
        matches!(findings.as_slice(), [Finding::ConflictCopy { .. }]),
        "the conflict copy is reported by the run that applied it"
    );
    assert_eq!(
        count_named(&b.port.tree(), "conflict"),
        1,
        "applied locally"
    );
    assert_eq!(head_seq(&hub), Some(2), "nothing was pushed");
    assert_eq!(b.last_seq(), 1, "last_seq is not ahead of what was stored");

    // the next run merges again and changes nothing locally: still one copy
    proxy.fail_puts.store(false, Ordering::SeqCst);
    let tree_before = b.port.tree();
    let Outcome::Synced { findings, seq } = b.run() else {
        panic!("expected a synced run");
    };
    assert_eq!(seq, 3);
    assert!(
        !findings
            .iter()
            .any(|f| matches!(f, Finding::ConflictCopy { .. })),
        "the second run adds no second copy and no second notice"
    );
    assert_eq!(
        b.port.tree(),
        tree_before,
        "no second copy, no duplicate nodes"
    );
    assert_eq!(count_named(&b.port.tree(), "conflict"), 1);
    assert!(matches!(a.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(a.port.tree(), b.port.tree());
    assert_eq!(names(&a.port.tree()).len(), 2);
}

#[test]
fn a_local_change_during_the_run_restarts_it_and_the_edit_survives() {
    let hub = start();
    let (a, b) = pair(&hub);
    a.port.add_connection("from A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    // the user adds a node after the engine read the local data and before it applies
    b.port.set_hook(|port| {
        port.add_connection("typed meanwhile");
    });
    assert!(matches!(b.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(names(&b.port.tree()), ["from A", "typed meanwhile"]);
    assert!(matches!(a.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(names(&a.port.tree()), ["from A", "typed meanwhile"]);
}

#[test]
fn a_push_conflict_restarts_the_run_and_a_notice_is_not_shown_twice() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    a.port.rename(id, "srv-a");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    b.port.rename(id, "srv-b");
    // while B applies its merge, A pushes again: B's push is refused and its run starts over
    let a2 = a.clone();
    b.port.set_hook(move |_| {
        a2.port.add_connection("late on A");
        assert!(matches!(a2.run(), Outcome::Synced { seq: 3, .. }));
    });
    let Outcome::Synced { findings, seq } = b.run() else {
        panic!("expected a synced run");
    };
    assert_eq!(seq, 4);
    // the first attempt applied the copy and its push was refused; the retry sees the copy as
    // an ordinary local node, so the notice must come from the first attempt, exactly once
    let copies = findings
        .iter()
        .filter(|f| matches!(f, Finding::ConflictCopy { .. }))
        .count();
    assert_eq!(copies, 1, "findings: {findings:?}");
    for (i, f) in findings.iter().enumerate() {
        assert!(!findings[..i].contains(f), "a finding is repeated");
    }
    assert_eq!(count_named(&b.port.tree(), "conflict"), 1);
    assert_eq!(count_named(&b.port.tree(), "late on A"), 1);
    assert!(matches!(a.run(), Outcome::Synced { seq: 4, .. }));
    assert_eq!(a.port.tree(), b.port.tree());
}

// ---- hub misbehaviour -------------------------------------------------------------------------

#[test]
fn a_stale_head_below_last_seq_is_hub_rolled_back_and_nothing_is_written() {
    let hub = start();
    let (a, b) = pair(&hub);
    let group = group_of(&a);
    let key = key_of(&a, group);
    a.port.add_connection("x");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));

    // a hub restored from an older backup: it holds seq 1
    let old = start();
    put_head(&old, &key, group, 1, 1, Tree::new());
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = old.client();
    b.port.add_connection("local edit");
    let before = b.untouched();
    let Outcome::HubRolledBack { found, seen } = b.run() else {
        panic!("expected a rollback");
    };
    assert_eq!((found, seen), (1, 2));
    b.assert_untouched(&before);
    assert_eq!(head_seq(&old), Some(1));
}

#[test]
fn an_empty_hub_after_sync_is_hub_rolled_back_and_reseed_rebuilds_it() {
    let hub = start();
    let (a, b) = pair(&hub);
    a.port.add_connection("kept");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));

    let fresh = start();
    let (mut a, mut b) = (
        Rc::try_unwrap(a).ok().unwrap(),
        Rc::try_unwrap(b).ok().unwrap(),
    );
    a.hub = fresh.client();
    b.hub = fresh.client();
    let before = b.untouched();
    let Outcome::HubRolledBack { found, seen } = b.run() else {
        panic!("expected a rollback");
    };
    assert_eq!((found, seen), (0, 2));
    b.assert_untouched(&before);
    assert_eq!(head_seq(&fresh), None);

    b.engine().reseed().unwrap();
    assert_eq!(head_seq(&fresh), Some(3));
    assert_eq!(b.last_seq(), 3);
    assert_eq!(b.state.load_base().unwrap().unwrap().seq, 3);
    assert!(matches!(a.run(), Outcome::NothingToDo));
    assert_eq!(a.last_seq(), 3);
    a.port.add_connection("after rebuild");
    assert!(matches!(a.run(), Outcome::Synced { seq: 4, .. }));
    assert!(matches!(b.run(), Outcome::Synced { seq: 4, .. }));
    assert_eq!(b.port.tree(), a.port.tree());
}

#[test]
fn reseed_above_a_newer_head_uses_the_right_expectation() {
    let hub = start();
    let (a, b) = pair(&hub);
    a.port.add_connection("only on A");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    // B never saw seq 2; a re-seed from B goes above the head it finds
    b.port.add_connection("only on B");
    b.engine().reseed().unwrap();
    assert_eq!(head_seq(&hub), Some(3));
    assert_eq!(b.last_seq(), 3);
}

#[test]
fn an_unreachable_hub_is_offline_with_the_pending_count() {
    let hub = start();
    let (a, b) = pair(&hub);
    drop(a);
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = HubClient::new(&cfg(&format!("http://{dead}"))).unwrap();
    assert_eq!(b.engine().pending_changes().unwrap(), 0);
    b.port.add_connection("one");
    b.port.add_connection("two");
    b.port.create_vault();
    let before = b.untouched();
    let Outcome::Offline {
        pending_changes, ..
    } = b.run()
    else {
        panic!("expected offline");
    };
    assert_eq!(pending_changes, 3, "two nodes and the vault");
    assert_eq!(b.engine().pending_changes().unwrap(), 3);
    b.assert_untouched(&before);
}

#[test]
fn a_hub_that_serves_a_different_group_is_wrong_group() {
    let hub = start();
    let (_a, b) = pair(&hub);
    let group = group_of(&b);
    let key = key_of(&b, group);
    let other = start();
    put_head(&other, &key, Uuid::from_u128(77), 5, 5, Tree::new());
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = other.client();
    b.port.add_connection("local");
    let before = b.untouched();
    assert_eq!(b.run_err().code(), "wrong_group");
    b.assert_untouched(&before);
}

#[test]
fn a_head_with_a_tree_deeper_than_60_is_refused_and_writes_nothing() {
    let hub = start();
    let (_a, b) = pair(&hub);
    let group = group_of(&b);
    let key = key_of(&b, group);
    let evil = start();
    put_head(&evil, &key, group, 5, 5, nested(61));
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = evil.client();
    let before = b.untouched();
    assert_eq!(b.run_err().code(), "snapshot_invalid");
    b.assert_untouched(&before);
    assert_eq!(head_seq(&evil), Some(5));
}

#[test]
fn a_head_with_more_than_100000_nodes_is_refused_and_writes_nothing() {
    let hub = start();
    let (_a, b) = pair(&hub);
    let group = group_of(&b);
    let key = key_of(&b, group);
    let big = start();
    let nodes: Vec<Node> = (0..100_001).map(|_| connection("n")).collect();
    put_head(&big, &key, group, 5, 5, Tree::from_children(nodes));
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = big.client();
    let before = b.untouched();
    assert_eq!(b.run_err().code(), "snapshot_invalid");
    b.assert_untouched(&before);
}

#[test]
fn a_snapshot_whose_group_or_seq_differs_from_its_envelope_is_refused() {
    let hub = start();
    let (_a, b) = pair(&hub);
    let group = group_of(&b);
    let key = key_of(&b, group);
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    // the envelope says seq 5, the authenticated snapshot says 4
    let seq_lie = start();
    put_head(&seq_lie, &key, group, 5, 4, Tree::new());
    b.hub = seq_lie.client();
    let before = b.untouched();
    assert_eq!(b.run_err().code(), "snapshot_invalid");
    b.assert_untouched(&before);
    // the envelope says this group, the snapshot another
    let group_lie = start();
    let snapshot = Snapshot {
        format_version: 1,
        group_id: Uuid::from_u128(5),
        seq: 5,
        writer: DeviceInfo {
            id: Uuid::from_u128(9),
            name: "other".to_owned(),
        },
        written_at: DATE.to_owned(),
        connections: Tree::new(),
        vault: None,
    };
    let bytes = seal(&key, group, 5, &snapshot.to_plaintext().unwrap()).unwrap();
    group_lie
        .client()
        .put_head(Expected::Empty, 5, &bytes)
        .unwrap();
    b.hub = group_lie.client();
    assert_eq!(b.run_err().code(), "snapshot_invalid");
    b.assert_untouched(&before);
}

#[test]
fn a_merged_result_over_64_mib_is_snapshot_too_large() {
    let hub = start();
    let (a, b) = pair(&hub);
    // the head has a change that A would have to apply, so a missing size check would be
    // visible in A's local data
    b.port.add_connection("pushed by B");
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    // a vault file that no envelope can carry (the engine only measures it; it is checked for
    // size before it is checked for being parseable)
    *a.port.fake_vault.lock().unwrap() = Some(vec![7u8; 65 * 1024 * 1024]);
    let before = a.untouched();
    assert_eq!(a.run_err().code(), "snapshot_too_large");
    a.assert_untouched(&before);
    assert!(names(&a.port.tree()).is_empty(), "nothing was applied");
    assert_eq!(head_seq(&hub), Some(2));
}

#[test]
fn a_local_vault_file_that_does_not_parse_is_refused_before_any_apply_or_push() {
    let hub = start();
    let (a, b) = pair(&hub);
    b.port.add_connection("pushed by B");
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));
    // A's vault file is damaged: it must never reach the hub
    *a.port.fake_vault.lock().unwrap() = Some(vec![1, 2, 3, 4]);
    let before = a.untouched();
    let err = a.run_err();
    assert!(matches!(&err, SyncError::LocalFailed(c) if c == "vault_unreadable"));
    a.assert_untouched(&before);
    assert!(names(&a.port.tree()).is_empty(), "nothing was applied");
    assert_eq!(head_seq(&hub), Some(2), "nothing was pushed");
}

#[test]
fn offline_after_an_applied_merge_returns_the_findings() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    let proxy = start_proxy(hub.addr);
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = HubClient::new(&cfg(&format!("http://{}", proxy.addr))).unwrap();
    a.port.rename(id, "srv-a");
    b.port.rename(id, "srv-b");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));
    proxy.fail_puts.store(true, Ordering::SeqCst);
    let Outcome::Offline {
        pending_changes,
        findings,
    } = b.run()
    else {
        panic!("expected offline");
    };
    assert!(matches!(
        findings.as_slice(),
        [Finding::ConflictCopy { .. }]
    ));
    assert!(pending_changes >= 1, "the unpushed copy is pending");
}

#[test]
fn errors_and_debug_output_name_no_secret() {
    let hub = start();
    let (a, _b) = pair(&hub);
    let group = group_of(&a);
    let key = key_of(&a, group);
    let shown = format!(
        "{:?} {:?} {}",
        SyncError::HubNotEmpty,
        a.port.read().unwrap(),
        SyncError::LocalFailed("x".into())
    );
    assert!(!shown.contains(SECRET));
    assert!(!shown.contains(&format!("{:?}", key.as_bytes())));
}

// ---- a push that was stored but not answered, and the attempt bound --------------------------

#[test]
fn a_push_the_hub_stored_but_never_answered_is_repaired_without_a_second_push() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    let proxy = start_proxy(hub.addr);
    let mut b = Rc::try_unwrap(b).ok().unwrap();
    b.hub = HubClient::new(&cfg(&format!("http://{}", proxy.addr))).unwrap();

    a.port.rename(id, "srv-a");
    b.port.rename(id, "srv-b");
    assert!(matches!(a.run(), Outcome::Synced { seq: 2, .. }));

    // B merges, applies and pushes; the hub stores seq 3 but B never sees the answer
    proxy.swallow_put_responses.store(true, Ordering::SeqCst);
    let Outcome::Offline { findings, .. } = b.run() else {
        panic!("expected offline");
    };
    assert!(matches!(
        findings.as_slice(),
        [Finding::ConflictCopy { .. }]
    ));
    assert_eq!(head_seq(&hub), Some(3), "the hub did store the push");
    assert_eq!(b.last_seq(), 1, "but this device never learned of it");
    assert_eq!(b.state.load_base().unwrap().unwrap().seq, 1);

    // the next run finds its own push as the head: nothing to push, nothing to apply, and the
    // state catches up
    proxy.swallow_put_responses.store(false, Ordering::SeqCst);
    let tree_before = b.port.tree();
    assert!(matches!(b.run(), Outcome::NothingToDo));
    assert_eq!(head_seq(&hub), Some(3), "no second push");
    assert_eq!(b.port.tree(), tree_before, "no second copy, no duplicate");
    assert_eq!(count_named(&b.port.tree(), "conflict"), 1);
    assert_eq!(b.last_seq(), 3);
    assert_eq!(b.state.load_base().unwrap().unwrap().seq, 3);
    assert!(matches!(a.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(a.port.tree(), b.port.tree());
    assert!(matches!(b.run(), Outcome::NothingToDo));
}

/// Makes the local data change under every apply (a user typing while a run is going).
fn edit_during_every_apply(port: &Port, counter: &Rc<Cell<usize>>) {
    let counter = counter.clone();
    port.set_hook(move |p| {
        counter.set(counter.get() + 1);
        p.add_connection("typed during the run");
        edit_during_every_apply(p, &counter);
    });
}

#[test]
fn a_run_starts_over_five_times_at_most_and_then_ends_in_local_changed() {
    let hub = start();
    let (a, b) = pair(&hub);
    b.port.add_connection("from B");
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));

    let attempts = Rc::new(Cell::new(0));
    edit_during_every_apply(&a.port, &attempts);
    let files_before = a.state_files();
    let err = a.run_err();
    assert_eq!(err.code(), "local_changed");
    assert_eq!(attempts.get(), 5, "exactly five attempts, not four or six");
    assert_eq!(a.state_files(), files_before, "no state was written");
    assert_eq!(head_seq(&hub), Some(2), "nothing was pushed");
    assert_eq!(count_named(&a.port.tree(), "typed during the run"), 5);
    assert_eq!(
        count_named(&a.port.tree(), "from B"),
        0,
        "nothing was applied"
    );

    // once the user stops typing the next run goes through
    a.port.hook.borrow_mut().take();
    assert!(matches!(a.run(), Outcome::Synced { seq: 3, .. }));
    assert_eq!(count_named(&a.port.tree(), "from B"), 1);
}

/// Lets the other device push a change under every apply, so this device's push is refused.
fn push_from_other_during_every_apply(port: &Port, other: &Rc<Device>, counter: &Rc<Cell<usize>>) {
    let (other, counter) = (other.clone(), counter.clone());
    port.set_hook(move |p| {
        counter.set(counter.get() + 1);
        other
            .port
            .add_connection(&format!("other {}", counter.get()));
        other.run();
        push_from_other_during_every_apply(p, &other, &counter);
    });
}

#[test]
fn five_refused_pushes_after_an_applied_merge_end_offline_with_the_findings_once() {
    let hub = start();
    let (a, b, id) = pair_with_server(&hub);
    a.port.rename(id, "srv-a");
    b.port.rename(id, "srv-b");
    assert!(matches!(b.run(), Outcome::Synced { seq: 2, .. }));

    let attempts = Rc::new(Cell::new(0));
    push_from_other_during_every_apply(&a.port, &b, &attempts);
    let Outcome::Offline { findings, .. } = a.run() else {
        panic!("expected offline after five refused pushes");
    };
    assert_eq!(attempts.get(), 5, "five attempts");
    assert_eq!(
        head_seq(&hub),
        Some(7),
        "the other device pushed five times"
    );
    assert!(
        matches!(findings.as_slice(), [Finding::ConflictCopy { .. }]),
        "the copy is reported once, not once per attempt: {findings:?}"
    );
    assert_eq!(a.last_seq(), 1, "this device pushed nothing");

    // the merge sits locally; the next run (no more interference) pushes it
    a.port.hook.borrow_mut().take();
    assert!(matches!(a.run(), Outcome::Synced { seq: 8, .. }));
    assert_eq!(count_named(&a.port.tree(), "conflict"), 1);
    assert!(matches!(b.run(), Outcome::Synced { seq: 8, .. }));
    assert_eq!(a.port.tree(), b.port.tree());
}

// ---- join and setup refusals -----------------------------------------------------------------

fn code_for(
    hub_url: &str,
    from: &fernjo_sync::pairing::PairingCode,
) -> fernjo_sync::pairing::PairingCode {
    fernjo_sync::pairing::PairingCode {
        url: hub_url.to_owned(),
        secret: from.secret.clone(),
        key: SyncKey::from_bytes(*from.key.as_bytes()),
        group: from.group,
        ca_pem: from.ca_pem.clone(),
    }
}

/// Everything a refused join must leave alone: local data, state files and the keychain.
fn assert_join_refused(b: &Device, code: &fernjo_sync::pairing::PairingCode, expected: &str) {
    let before = b.untouched();
    let err = b.engine().join(code, "B").unwrap_err();
    assert_eq!(err.code(), expected);
    b.assert_untouched(&before);
    assert!(b.state.load().unwrap().is_none(), "no sync.json");
    assert!(
        b.secrets.get(code.group).unwrap().is_none(),
        "no keychain item"
    );
}

#[test]
fn join_on_an_empty_hub_is_hub_empty_and_changes_nothing() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    let code = a.setup(&hub.url());
    let empty = start();
    let b = Device::new("B", &empty.url());
    b.port.add_connection("mine");
    assert_join_refused(&b, &code_for(&empty.url(), &code), "hub_empty");
}

#[test]
fn join_refuses_a_foreign_group_a_wrong_key_an_unreachable_hub_and_a_second_join() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    a.port.add_connection("web");
    let code = a.setup(&hub.url());
    let b = Device::new("B", &hub.url());
    b.port.add_connection("mine");

    let mut foreign = code_for(&hub.url(), &code);
    foreign.group = Uuid::from_u128(0xDEAD);
    assert_join_refused(&b, &foreign, "wrong_group");

    let mut wrong_key = code_for(&hub.url(), &code);
    wrong_key.key = SyncKey::from_bytes([9; 32]);
    assert_join_refused(&b, &wrong_key, "wrong_key_or_damaged");

    assert_join_refused(
        &b,
        &code_for("http://127.0.0.1:1", &code),
        "hub_unreachable",
    );

    // the right code works once; a second join is refused and changes nothing
    b.join(&code);
    let before = b.untouched();
    let err = b.engine().join(&code, "B again").unwrap_err();
    assert_eq!(err.code(), "already_paired");
    b.assert_untouched(&before);
}

#[test]
fn join_refuses_a_head_that_is_not_a_valid_snapshot_and_changes_nothing() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    let code = a.setup(&hub.url());
    // a second hub holds a head of the same group and key whose tree is nested too deeply
    let bad = start();
    put_head(&bad, &code.key, code.group, 1, 1, nested(61));
    let b = Device::new("B", &bad.url());
    b.port.add_connection("mine");
    assert_join_refused(&b, &code_for(&bad.url(), &code), "snapshot_invalid");
}

// ---- hub errors surface as errors, not as "offline" ---------------------------------------------

#[test]
fn a_hub_that_refuses_the_secret_is_an_error_not_offline_and_nothing_changes() {
    let hub = start();
    let (a, _b) = pair(&hub);
    let mut a = Rc::try_unwrap(a).ok().unwrap();
    a.hub = HubClient::new(&HubConfig {
        url: hub.url(),
        secret: SecretString::from("not the hub secret".to_owned()),
        ca_pem: None,
    })
    .unwrap();
    a.port.add_connection("local edit");
    let before = a.untouched();
    assert_eq!(a.run_err().code(), "hub_unauthorized");
    a.assert_untouched(&before);
    assert_eq!(
        a.engine().pending_changes().unwrap(),
        1,
        "the edit still waits"
    );
}

/// A TLS server whose certificate comes from a CA this client has never heard of.
fn untrusted_tls_hub() -> String {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};

    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let config = Arc::new(
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(cert.der().to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let config = Arc::clone(&config);
            thread::spawn(move || {
                if let Ok(conn) = ServerConnection::new(config) {
                    let mut tls = StreamOwned::new(conn, stream);
                    let mut byte = [0u8; 1];
                    let _ = tls.read(&mut byte);
                }
            });
        }
    });
    format!("https://127.0.0.1:{port}")
}

#[test]
fn a_hub_with_an_untrusted_certificate_is_an_error_not_offline_and_nothing_changes() {
    let hub = start();
    let (a, _b) = pair(&hub);
    let mut a = Rc::try_unwrap(a).ok().unwrap();
    a.hub = HubClient::new(&cfg(&untrusted_tls_hub())).unwrap();
    a.port.add_connection("local edit");
    let before = a.untouched();
    assert_eq!(a.run_err().code(), "hub_tls_untrusted");
    a.assert_untouched(&before);
}

// ---- setup refusals ----------------------------------------------------------------------------

/// A keychain double that records what the engine does with it.
#[derive(Default)]
struct RecordingSecrets {
    inner: MemorySyncSecrets,
    events: Mutex<Vec<String>>,
    fail_set: AtomicBool,
}

impl RecordingSecrets {
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl SyncSecrets for RecordingSecrets {
    fn get(&self, group: GroupId) -> Result<Option<(SecretString, SyncKey)>, SyncError> {
        self.inner.get(group)
    }
    fn set(
        &self,
        group: GroupId,
        hub_secret: &SecretString,
        key: &SyncKey,
    ) -> Result<(), SyncError> {
        self.events.lock().unwrap().push(format!("set {group}"));
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(SyncError::KeychainUnavailable("locked".to_owned()));
        }
        self.inner.set(group, hub_secret, key)
    }
    fn delete(&self, group: GroupId) -> Result<(), SyncError> {
        self.events.lock().unwrap().push(format!("delete {group}"));
        self.inner.delete(group)
    }
}

fn engine_with<'a>(d: &'a Device, secrets: &'a RecordingSecrets, hub: &'a HubClient) -> Engine<'a> {
    Engine {
        hub,
        state: &d.state,
        secrets,
        local: &d.port,
        ctx: MergeContext {
            device_name: d.name.clone(),
            date: DATE.to_owned(),
        },
    }
}

#[test]
fn setup_that_fails_to_push_deletes_the_secrets_it_stored_and_leaves_no_state() {
    let hub = start();
    let proxy = start_proxy(hub.addr);
    let url = format!("http://{}", proxy.addr);
    let a = Device::new("A", &url);
    a.port.add_connection("web");
    let secrets = RecordingSecrets::default();
    let client = HubClient::new(&cfg(&url)).unwrap();
    // the info and head checks pass, then the upload never reaches the hub
    proxy.fail_puts.store(true, Ordering::SeqCst);
    let before = a.untouched();
    let err = engine_with(&a, &secrets, &client)
        .setup(&cfg(&url), "A")
        .unwrap_err();
    assert_eq!(err.code(), "hub_unreachable");

    let events = secrets.events();
    assert_eq!(events.len(), 2, "{events:?}");
    let group = events[0].strip_prefix("set ").expect("stored first");
    assert_eq!(events[1], format!("delete {group}"), "and removed again");
    assert!(secrets.get(group.parse().unwrap()).unwrap().is_none());
    a.assert_untouched(&before);
    assert!(a.state.load().unwrap().is_none());
    assert_eq!(head_seq(&hub), None, "nothing is on the hub");
}

#[test]
fn setup_with_a_keychain_that_cannot_store_uploads_nothing() {
    let hub = start();
    let a = Device::new("A", &hub.url());
    let secrets = RecordingSecrets::default();
    secrets.fail_set.store(true, Ordering::SeqCst);
    let client = HubClient::new(&cfg(&hub.url())).unwrap();
    let before = a.untouched();
    let err = engine_with(&a, &secrets, &client)
        .setup(&cfg(&hub.url()), "A")
        .unwrap_err();
    assert_eq!(err.code(), "keychain_unavailable");
    assert_eq!(head_seq(&hub), None, "nothing was uploaded");
    a.assert_untouched(&before);
    assert!(a.state.load().unwrap().is_none());
}
