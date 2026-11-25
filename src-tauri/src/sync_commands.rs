//! Tauri commands for multi-device sync (M9) and the run logic the runner and the commands share.
//!
//! Like `commands.rs`: plain functions over `&AppState` (testable without Tauri) plus thin
//! wrappers in [`handlers`]. The commands that talk to the hub are `async fn`s that move the
//! work to a blocking thread and take the state from their `AppHandle` (the `import_apply`
//! pattern); none of them holds a lock while it waits for the network. The hub secret and the
//! sync key stay in Rust: they live in the OS keychain (`AppState.sync_secrets`), no response,
//! error or `Debug` output contains them, and `sync.json` holds none. The one exception is
//! [`sync_pairing_code`], an explicit reveal like the vault's.
//!
//! # Wire contract
//!
//! Commands (arguments in camelCase; every failure is a `{ code, message }` error):
//!
//! | command | arguments | result |
//! |---|---|---|
//! | `sync_status` | none | `SyncStatus` |
//! | `sync_setup` | `url`, `secret`, `caPem?`, `deviceName` | `SyncStatus` |
//! | `sync_pairing_code` | none | the pairing code (string) |
//! | `sync_join` | `code`, `deviceName`, `confirmReplace` | `SyncStatus` |
//! | `sync_now` | none | `SyncStatus` |
//! | `sync_reseed` | none | `SyncStatus` |
//! | `sync_edit_secret` | `secret` | `SyncStatus` |
//! | `sync_leave` | none | `SyncStatus` (state `off`; `error` `keychain_unavailable` when the keychain item could not be removed) |
//! | `sync_dismiss_findings` | none | nothing |
//!
//! `SyncStatus` (camelCase):
//! - `state`: `"off"` (not set up), `"ok"`, `"offline"` (hub unreachable, `pendingChanges` wait),
//!   `"waiting_for_unlock"` (both sides changed the vault and it is locked), `"rolled_back"` (the
//!   hub shows an older state than this device saw; `error` says which: `hub_rolled_back` for
//!   the sequence, `vault_rolled_back` when the run refused the head's vault as older than the
//!   base or this device's rollback mark, V5; "Re-seed" repairs both),
//!   `"error"` (the last run failed, `error` names the code) or `"needs_confirmation"` (only the
//!   answer of `sync_join` without `confirmReplace`).
//! - `lastSyncedAt`: RFC 3339 UTC of the last successful run since the app started, else `null`.
//! - `pendingChanges`: what `Engine::pending_changes` counts (nodes and the vault file that
//!   differ from the base); `0` when off.
//! - `deviceName`, `hubUrl`: from `sync.json`, `null` when off.
//! - `findings`: `{ id, name, kind, message }` for each merge notice not yet dismissed. `kind` is
//!   `conflictCopy`, `keptEdit`, `placementKept`, `moveUndone`, `entryConflictCopy`,
//!   `entryKeptEdit` or `passwordChangeDropped` (the master password set on this device was
//!   replaced by the other device's after both changed the vault; `name` is "Vault"); `id` is the
//!   node or entry id (the vault id for `passwordChangeDropped`), `message` is the sentence to
//!   show. They stay until `sync_dismiss_findings`.
//! - `error`: `{ code, message }` or `null`. Codes are the stable ones of `fernjo-sync`
//!   (`hub_unauthorized`, `hub_tls_untrusted`, `snapshot_invalid`, `state_unusable`,
//!   `keychain_unavailable`, ...); a refusal of the local side carries the code of the `Store`
//!   or `Vault` error (`changed_on_disk`, `rolled_back`, ...).
//! - `running`: a run is in progress (a second trigger is dropped, `sync_now` answers with this
//!   flag set instead of starting another run).
//! - `applied`: how many runs wrote remote changes into the local data since the app started;
//!   when it changes the tree and the vault status have to be fetched again.
//! - `confirmation`: only with `needs_confirmation`: `{ connections, folders, vaultEntries,
//!   hasVault }` of the local data a join would replace (`vaultEntries` is `null` while the
//!   vault is locked).
//!
//! Errors of note: `no_device_folder` (no device-local folder: setup and join are refused),
//! `not_paired`, `already_paired`, `sync_busy` (a run is in progress), `restart_required`,
//! `invalid_device_name`, plus the codes above.
//!
//! Event `sync-status` (payload `SyncStatus`): emitted after every run of the background runner,
//! so the UI needs no polling.

use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use fernjo_sync::SyncError;
use fernjo_sync::client::{HubClient, HubConfig};
use fernjo_sync::engine::{Engine, Outcome};
use fernjo_sync::merge::report::describe;
use fernjo_sync::merge::{Finding, MergeContext};
use fernjo_sync::pairing::PairingCode;
use fernjo_sync::ports::LocalPort;
use fernjo_sync::state::StateStore;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use crate::commands::{CommandError, ensure_writable};
use crate::state::{AppState, VaultState};
use crate::sync_port::{AppPort, count_nodes};

/// The longest device name accepted.
const MAX_DEVICE_NAME_CHARS: usize = 100;

/// What the last run left behind. Everything here is shown to the user, so none of it is secret.
#[derive(Debug, Default)]
pub struct SyncRuntime {
    /// This device has a `sync.json` (refreshed by every command that can change it).
    pub paired: bool,
    /// A run is in progress.
    pub running: bool,
    last: LastOutcome,
    last_synced_at: Option<String>,
    /// Notices not yet dismissed, de-duplicated by equality.
    findings: Vec<Finding>,
    error: Option<StatusError>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum LastOutcome {
    /// No run since the app started.
    #[default]
    Unknown,
    Ok,
    Offline,
    WaitingForUnlock,
    RolledBack {
        found: u64,
        seen: u64,
    },
    /// The run refused the head's vault as older than this device has seen (V5: a head below the
    /// base's generation, or a file below this device's rollback mark; the engine's or the
    /// vault's `rolled_back`). Shown like a rolled-back hub, so the user is offered Re-seed,
    /// which pushes this device's newer vault instead of merging the older one.
    VaultRolledBack,
}

impl SyncRuntime {
    fn add_findings(&mut self, findings: Vec<Finding>) {
        for finding in findings {
            if !self.findings.contains(&finding) {
                self.findings.push(finding);
            }
        }
    }

    /// Forgets the last run's results. `running` stays: the command that resets still holds
    /// the run slot (its guard clears the flag).
    fn reset(&mut self) {
        *self = SyncRuntime {
            running: self.running,
            ..SyncRuntime::default()
        };
    }
}

fn runtime(state: &AppState) -> MutexGuard<'_, SyncRuntime> {
    state
        .sync
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// An error as the status shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusError {
    pub code: String,
    pub message: String,
}

/// A merge notice for the UI: ids and names only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindingView {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub message: String,
}

impl FindingView {
    fn of(finding: &Finding) -> FindingView {
        let (id, name, kind) = match finding {
            Finding::ConflictCopy { copy, name, .. } => {
                (copy.to_string(), name.as_str(), "conflictCopy")
            }
            Finding::KeptEdit { node, name } => (node.to_string(), name.as_str(), "keptEdit"),
            Finding::PlacementKept { node, name } => {
                (node.to_string(), name.as_str(), "placementKept")
            }
            Finding::MoveUndone { node, name } => (node.to_string(), name.as_str(), "moveUndone"),
            Finding::EntryConflictCopy { copy, name, .. } => {
                (copy.to_string(), name.as_str(), "entryConflictCopy")
            }
            Finding::EntryKeptEdit { entry, name } => {
                (entry.to_string(), name.as_str(), "entryKeptEdit")
            }
            Finding::PasswordChangeDropped { vault } => {
                (vault.to_string(), "Vault", "passwordChangeDropped")
            }
        };
        FindingView {
            id,
            name: name.to_owned(),
            kind,
            message: describe(finding),
        }
    }
}

/// What a join would replace on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinConfirmation {
    pub connections: usize,
    pub folders: usize,
    /// `None` while the vault is locked (the entries cannot be counted).
    pub vault_entries: Option<usize>,
    pub has_vault: bool,
}

/// The answer of every sync command that reports state (see the wire contract above).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub state: &'static str,
    pub last_synced_at: Option<String>,
    pub pending_changes: usize,
    pub device_name: Option<String>,
    pub hub_url: Option<String>,
    pub findings: Vec<FindingView>,
    pub error: Option<StatusError>,
    pub running: bool,
    pub applied: u64,
    pub confirmation: Option<JoinConfirmation>,
}

fn error(code: &str, message: impl Into<String>) -> CommandError {
    CommandError {
        code: code.to_owned(),
        message: message.into(),
    }
}

/// The stable code of a sync error as commands and the status report it: a refusal of the local
/// side carries the code of the `Store` or `Vault` error behind it.
fn code_of(e: &SyncError) -> String {
    match e {
        SyncError::LocalFailed(code) => code.clone(),
        other => other.code().to_owned(),
    }
}

impl From<SyncError> for CommandError {
    fn from(e: SyncError) -> Self {
        CommandError {
            code: code_of(&e),
            message: e.to_string(),
        }
    }
}

fn status_error(e: &SyncError) -> StatusError {
    StatusError {
        code: code_of(e),
        message: e.to_string(),
    }
}

fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// The device-local sync files, or `no_device_folder`.
fn state_store(state: &AppState) -> Result<StateStore, CommandError> {
    state
        .device_dir
        .clone()
        .map(StateStore::new)
        .ok_or_else(|| {
            error(
                "no_device_folder",
                "this system has no device-local folder, so sync cannot be set up",
            )
        })
}

/// A client that never talks: `Engine::pending_changes`, `setup` and `join` need a `HubClient`
/// in the struct but use only the local side (setup and join build their own client).
/// `HubClient::offline` has no address, so it does not depend on which addresses this build
/// accepts (release builds refuse plain `http://`), and loads no certificate roots.
fn idle_hub() -> HubClient {
    HubClient::offline()
}

fn pending_changes(state: &AppState, store: &StateStore) -> Result<usize, SyncError> {
    let hub = idle_hub();
    let port = AppPort(state);
    Engine {
        hub: &hub,
        state: store,
        secrets: &*state.sync_secrets,
        local: &port,
        ctx: MergeContext {
            device_name: String::new(),
            date: String::new(),
        },
    }
    .pending_changes()
}

/// The status as it is now. Never talks to the network.
pub fn build_status(state: &AppState) -> SyncStatus {
    let loaded = match &state.device_dir {
        None => Ok(None),
        Some(dir) => StateStore::new(dir.clone()).load(),
    };
    let (mut pending, mut error_now, st) = (0, None, None::<fernjo_sync::state::SyncState>);
    let st = match loaded {
        Ok(st) => st,
        Err(e) => {
            error_now = Some(status_error(&e));
            st
        }
    };
    if let (Some(_), Some(dir)) = (&st, &state.device_dir) {
        match pending_changes(state, &StateStore::new(dir.clone())) {
            Ok(n) => pending = n,
            Err(e) => error_now = Some(status_error(&e)),
        }
    }
    let mut rt = runtime(state);
    rt.paired = st.is_some();
    let mut app_state = if st.is_none() && error_now.is_none() {
        "off"
    } else {
        match rt.last {
            LastOutcome::Unknown | LastOutcome::Ok => "ok",
            LastOutcome::Offline => "offline",
            LastOutcome::WaitingForUnlock => "waiting_for_unlock",
            LastOutcome::RolledBack { found, seen } => {
                if error_now.is_none() {
                    error_now = Some(StatusError {
                        code: "hub_rolled_back".to_owned(),
                        message: format!(
                            "the hub shows state {found} but this device has seen {seen}; \
                             re-seed the hub from this device if that is expected"
                        ),
                    });
                }
                "rolled_back"
            }
            LastOutcome::VaultRolledBack => {
                if error_now.is_none() {
                    error_now = Some(StatusError {
                        code: "vault_rolled_back".to_owned(),
                        message: "the hub holds an older vault than this device has seen;                                   re-seed the hub from this device if that is expected"
                            .to_owned(),
                    });
                }
                "rolled_back"
            }
        }
    };
    let error_shown = error_now.or_else(|| rt.error.clone());
    if error_shown.is_some() && app_state != "rolled_back" {
        app_state = "error";
    }
    SyncStatus {
        state: app_state,
        last_synced_at: rt.last_synced_at.clone(),
        pending_changes: pending,
        device_name: st.as_ref().map(|s| s.device.name.clone()),
        hub_url: st.as_ref().map(|s| s.url.clone()),
        findings: rt.findings.iter().map(FindingView::of).collect(),
        error: error_shown,
        running: rt.running,
        applied: state.sync_applied.load(std::sync::atomic::Ordering::SeqCst),
        confirmation: None,
    }
}

/// Whether anything waits to be pushed (`false` when off or when it cannot be told).
pub fn has_pending_changes(state: &AppState) -> bool {
    state
        .device_dir
        .clone()
        .map(StateStore::new)
        .is_some_and(|store| pending_changes(state, &store).is_ok_and(|n| n > 0))
}

/// Re-reads `sync.json` to learn whether this device is set up (called at startup by the
/// runner, and by the commands that change it).
pub fn refresh_paired(state: &AppState) {
    let paired = state
        .device_dir
        .clone()
        .map(StateStore::new)
        .is_some_and(|s| matches!(s.load(), Ok(Some(_))));
    runtime(state).paired = paired;
}

/// Holds the "a run is in progress" flag; dropping it clears the flag.
pub struct RunGuard<'a>(&'a AppState);

impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        runtime(self.0).running = false;
    }
}

/// Claims the single run slot, or `None` while another run (or a setup, join, leave or re-seed)
/// has it.
pub fn try_begin(state: &AppState) -> Option<RunGuard<'_>> {
    let mut rt = runtime(state);
    if rt.running {
        None
    } else {
        rt.running = true;
        Some(RunGuard(state))
    }
}

fn begin(state: &AppState) -> Result<RunGuard<'_>, CommandError> {
    try_begin(state).ok_or_else(|| error("sync_busy", "a sync run is in progress"))
}

/// One engine run over `port`. The hub client is built from `sync.json` and the keychain; the
/// engine runs with no lock held (the port takes and releases its own).
fn run_engine(state: &AppState, port: &dyn LocalPort) -> Result<Outcome, SyncError> {
    let dir = state.device_dir.clone().ok_or(SyncError::NotPaired)?;
    let store = StateStore::new(dir);
    let st = store.load()?.ok_or(SyncError::NotPaired)?;
    let (secret, _key) = state.sync_secrets.get(st.group)?.ok_or_else(|| {
        SyncError::KeychainUnavailable("the sync key is not in the keychain".to_owned())
    })?;
    let hub = HubClient::new(&HubConfig {
        url: st.url.clone(),
        secret,
        ca_pem: st.ca_pem.clone(),
    })?;
    Engine {
        hub: &hub,
        state: &store,
        secrets: &*state.sync_secrets,
        local: port,
        ctx: MergeContext {
            device_name: st.device.name.clone(),
            date: crate::import_commands::today_local(),
        },
    }
    .run_once()
}

/// Remembers what a run reported: findings are kept until dismissed (also those of an offline
/// run whose merge was already applied), an error replaces the last one, any other outcome
/// clears it.
fn record(state: &AppState, result: Result<Outcome, SyncError>) {
    let mut rt = runtime(state);
    match result {
        Ok(Outcome::Synced { findings, .. }) => {
            rt.add_findings(findings);
            rt.last = LastOutcome::Ok;
            rt.last_synced_at = Some(now_utc());
            rt.error = None;
        }
        Ok(Outcome::NothingToDo) => {
            rt.last = LastOutcome::Ok;
            rt.last_synced_at = Some(now_utc());
            rt.error = None;
        }
        Ok(Outcome::Offline { findings, .. }) => {
            rt.add_findings(findings);
            rt.last = LastOutcome::Offline;
            rt.error = None;
        }
        Ok(Outcome::WaitingForUnlock) => {
            rt.last = LastOutcome::WaitingForUnlock;
            rt.error = None;
        }
        Ok(Outcome::HubRolledBack { found, seen }) => {
            rt.last = LastOutcome::RolledBack { found, seen };
            rt.error = None;
        }
        // the vault refusal of V5 repeats on every run until the hub is re-seeded: give it the
        // rolled-back state (and its Re-seed action), not a plain error
        Err(SyncError::LocalFailed(code)) if code == "rolled_back" => {
            rt.last = LastOutcome::VaultRolledBack;
            rt.error = None;
        }
        Err(e) => rt.error = Some(status_error(&e)),
    }
}

/// A run for the background triggers: `false` when another run had the slot or the device is
/// not set up (nothing happens then).
pub fn run_background(state: &AppState) -> bool {
    run_background_with(state, &AppPort(state))
}

pub(crate) fn run_background_with(state: &AppState, port: &dyn LocalPort) -> bool {
    if !runtime(state).paired {
        return false;
    }
    let Some(_guard) = try_begin(state) else {
        return false;
    };
    let result = run_engine(state, port);
    record(state, result);
    true
}

/// The final push when the window closes: like [`run_background`], but a run that already holds
/// the slot is waited for (at most `budget`) instead of making the close skip the push. Once
/// the slot is free it runs only when something still waits (the run that was in flight may have
/// pushed everything). `true` when it ran. Blocks the calling thread, so it belongs on a worker,
/// never on the UI thread.
pub fn push_before_close(state: &AppState, budget: Duration) -> bool {
    push_before_close_with(state, &AppPort(state), budget)
}

pub(crate) fn push_before_close_with(
    state: &AppState,
    port: &dyn LocalPort,
    budget: Duration,
) -> bool {
    if !runtime(state).paired {
        return false;
    }
    let deadline = Instant::now() + budget;
    loop {
        if let Some(_guard) = try_begin(state) {
            if !has_pending_changes(state) {
                return false;
            }
            let result = run_engine(state, port);
            record(state, result);
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// ---- commands ---------------------------------------------------------------------------

pub fn sync_status(state: &AppState) -> Result<SyncStatus, CommandError> {
    Ok(build_status(state))
}

fn check_device_name(name: &str) -> Result<String, CommandError> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_DEVICE_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(error(
            "invalid_device_name",
            "the device name must be 1 to 100 characters without control characters",
        ));
    }
    Ok(name.to_owned())
}

/// Sets this device up as the first of a new group: generates the key, uploads the local data.
/// The pairing code is fetched afterwards with [`sync_pairing_code`].
pub fn sync_setup(
    state: &AppState,
    url: String,
    secret: SecretString,
    ca_pem: Option<String>,
    device_name: String,
) -> Result<SyncStatus, CommandError> {
    ensure_writable(state)?;
    let store = state_store(state)?;
    let device_name = check_device_name(&device_name)?;
    let _guard = begin(state)?;
    let hub = idle_hub();
    let port = AppPort(state);
    Engine {
        hub: &hub,
        state: &store,
        secrets: &*state.sync_secrets,
        local: &port,
        ctx: MergeContext {
            device_name: device_name.clone(),
            date: crate::import_commands::today_local(),
        },
    }
    .setup(
        &HubConfig {
            url,
            secret,
            ca_pem,
        },
        &device_name,
    )?;
    {
        let mut rt = runtime(state);
        rt.reset();
        rt.paired = true;
        rt.last = LastOutcome::Ok;
        rt.last_synced_at = Some(now_utc());
    }
    Ok(build_status(state))
}

/// The pairing code of this group: URL, hub secret, sync key, group id and the optional CA. An
/// explicit reveal (the one sync response that carries the secrets).
pub fn sync_pairing_code(state: &AppState) -> Result<String, CommandError> {
    let store = state_store(state).map_err(|_| error("not_paired", "this device is not set up"))?;
    let st = store.load()?.ok_or(SyncError::NotPaired)?;
    let (secret, key) = state.sync_secrets.get(st.group)?.ok_or_else(|| {
        SyncError::KeychainUnavailable("the sync key is not in the keychain".to_owned())
    })?;
    Ok(PairingCode {
        url: st.url,
        secret,
        key,
        group: st.group,
        ca_pem: st.ca_pem,
    }
    .encode())
}

fn join_confirmation(state: &AppState) -> Result<JoinConfirmation, CommandError> {
    let (has_vault, vault_entries) = {
        let guard = state.vault.lock().map_err(|_| CommandError::internal())?;
        match &*guard {
            VaultState::None => (false, None),
            VaultState::Unusable { .. } => (true, None),
            VaultState::Open(v) if v.is_unlocked() => (true, v.list().ok().map(|l| l.len())),
            VaultState::Open(_) => (true, None),
        }
    };
    let (connections, folders) =
        crate::commands::with_store(state, |store| Ok(count_nodes(store.tree().children())))?;
    Ok(JoinConfirmation {
        connections,
        folders,
        vault_entries,
        has_vault,
    })
}

/// Joins the group of a pairing code; the head replaces the local connections and vault.
/// Without `confirm_replace` nothing is changed and the answer is `needs_confirmation` with the
/// counts that would be lost. With it the open sessions are closed first (they use the data being
/// replaced), then the join runs.
pub fn sync_join(
    state: &AppState,
    code: &SecretString,
    device_name: String,
    confirm_replace: bool,
) -> Result<SyncStatus, CommandError> {
    let code = PairingCode::decode(code.expose_secret())?;
    ensure_writable(state)?;
    let store = state_store(state)?;
    let device_name = check_device_name(&device_name)?;
    if store.load()?.is_some() {
        return Err(SyncError::AlreadyPaired.into());
    }
    if !confirm_replace {
        let mut status = build_status(state);
        status.state = "needs_confirmation";
        status.confirmation = Some(join_confirmation(state)?);
        return Ok(status);
    }
    let _guard = begin(state)?;
    state.sessions.close_all();
    let hub = idle_hub();
    let port = AppPort(state);
    Engine {
        hub: &hub,
        state: &store,
        secrets: &*state.sync_secrets,
        local: &port,
        ctx: MergeContext {
            device_name: device_name.clone(),
            date: crate::import_commands::today_local(),
        },
    }
    .join(&code, &device_name)?;
    {
        let mut rt = runtime(state);
        rt.reset();
        rt.paired = true;
        rt.last = LastOutcome::Ok;
        rt.last_synced_at = Some(now_utc());
    }
    Ok(build_status(state))
}

/// Runs a sync now. While another run is in progress the status is returned with `running`
/// set and no second run starts.
pub fn sync_now(state: &AppState) -> Result<SyncStatus, CommandError> {
    ensure_writable(state)?;
    let store = state_store(state).map_err(|_| error("not_paired", "this device is not set up"))?;
    if store.load()?.is_none() {
        return Err(SyncError::NotPaired.into());
    }
    runtime(state).paired = true;
    let _ran = run_background(state);
    Ok(build_status(state))
}

/// Pushes the local state as a new head above everything seen (after `rolled_back`, or to rebuild
/// a lost hub). The caller confirms in the UI.
pub fn sync_reseed(state: &AppState) -> Result<SyncStatus, CommandError> {
    ensure_writable(state)?;
    let store = state_store(state).map_err(|_| error("not_paired", "this device is not set up"))?;
    if store.load()?.is_none() {
        return Err(SyncError::NotPaired.into());
    }
    let guard = begin(state)?;
    let hub = {
        let st = store.load()?.ok_or(SyncError::NotPaired)?;
        let (secret, _) = state.sync_secrets.get(st.group)?.ok_or_else(|| {
            SyncError::KeychainUnavailable("the sync key is not in the keychain".to_owned())
        })?;
        HubClient::new(&HubConfig {
            url: st.url.clone(),
            secret,
            ca_pem: st.ca_pem.clone(),
        })?
    };
    let port = AppPort(state);
    Engine {
        hub: &hub,
        state: &store,
        secrets: &*state.sync_secrets,
        local: &port,
        ctx: MergeContext {
            device_name: String::new(),
            date: crate::import_commands::today_local(),
        },
    }
    .reseed()?;
    {
        let mut rt = runtime(state);
        rt.last = LastOutcome::Ok;
        rt.last_synced_at = Some(now_utc());
        rt.error = None;
    }
    drop(guard);
    Ok(build_status(state))
}

/// Replaces only the hub secret in the keychain (the sync key stays), after the hub accepted the
/// new one: a wrong secret is `hub_unauthorized` and nothing is stored.
pub fn sync_edit_secret(
    state: &AppState,
    secret: SecretString,
) -> Result<SyncStatus, CommandError> {
    let store = state_store(state).map_err(|_| error("not_paired", "this device is not set up"))?;
    let st = store.load()?.ok_or(SyncError::NotPaired)?;
    let (_old, key) = state.sync_secrets.get(st.group)?.ok_or_else(|| {
        SyncError::KeychainUnavailable("the sync key is not in the keychain".to_owned())
    })?;
    if secret.expose_secret().is_empty() {
        return Err(SyncError::HubUnauthorized.into());
    }
    HubClient::new(&HubConfig {
        url: st.url.clone(),
        secret: secret.clone(),
        ca_pem: st.ca_pem.clone(),
    })?
    .info()?;
    state.sync_secrets.set(st.group, &secret, &key)?;
    runtime(state).error = None;
    Ok(build_status(state))
}

/// Leaves sync: removes the keychain item, `sync.json` and the base; the local data stays. When
/// the keychain refuses the delete, the rest is still removed and the answer (state `off`) carries
/// `error` `keychain_unavailable` naming the item left behind.
pub fn sync_leave(state: &AppState) -> Result<SyncStatus, CommandError> {
    let store = state_store(state).map_err(|_| error("not_paired", "this device is not set up"))?;
    let _guard = begin(state)?;
    // A keychain that refuses the delete must not keep the device in sync: the state and the base
    // are cleared anyway and the leftover item is named in the answer.
    let mut leftover = None;
    match store.load() {
        Ok(Some(st)) => {
            if state.sync_secrets.delete(st.group).is_err() {
                leftover = Some(st.group);
            }
        }
        Ok(None) => return Err(SyncError::NotPaired.into()),
        // An unusable `sync.json` can still be left; its group (and so its keychain item) is
        // unknown then.
        Err(_) => {}
    }
    store.clear()?;
    runtime(state).reset();
    let mut status = build_status(state);
    if let Some(group) = leftover {
        status.error = Some(StatusError {
            code: "keychain_unavailable".to_owned(),
            message: format!(
                "This device left sync, but the keychain item with the sync keys could not be                  removed. Delete the item named \"{}\" (service \"fernjo\") from your OS                  keychain by hand.",
                fernjo_sync::secrets::keychain_user(group)
            ),
        });
    }
    Ok(status)
}

/// Forgets the merge notices the user has seen.
pub fn sync_dismiss_findings(state: &AppState) -> Result<(), CommandError> {
    runtime(state).findings.clear();
    Ok(())
}

/// The `#[tauri::command]` wrappers registered in `run()`.
pub mod handlers {
    use super::*;
    use tauri::{AppHandle, Manager, State};

    async fn blocking<T: Send + 'static>(
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, CommandError> {
        tauri::async_runtime::spawn_blocking(job)
            .await
            .map_err(|_| CommandError::internal())
    }

    #[tauri::command(async)]
    pub fn sync_status(state: State<'_, AppState>) -> Result<SyncStatus, CommandError> {
        super::sync_status(&state)
    }

    #[tauri::command]
    pub async fn sync_setup(
        app: AppHandle,
        url: String,
        secret: String,
        ca_pem: Option<String>,
        device_name: String,
    ) -> Result<SyncStatus, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::sync_setup(&state, url, SecretString::from(secret), ca_pem, device_name)
        })
        .await?
    }

    #[tauri::command(async)]
    pub fn sync_pairing_code(state: State<'_, AppState>) -> Result<String, CommandError> {
        super::sync_pairing_code(&state)
    }

    #[tauri::command]
    pub async fn sync_join(
        app: AppHandle,
        code: String,
        device_name: String,
        confirm_replace: bool,
    ) -> Result<SyncStatus, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::sync_join(
                &state,
                &SecretString::from(code),
                device_name,
                confirm_replace,
            )
        })
        .await?
    }

    #[tauri::command]
    pub async fn sync_now(app: AppHandle) -> Result<SyncStatus, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::sync_now(&state)
        })
        .await?
    }

    #[tauri::command]
    pub async fn sync_reseed(app: AppHandle) -> Result<SyncStatus, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::sync_reseed(&state)
        })
        .await?
    }

    #[tauri::command]
    pub async fn sync_edit_secret(
        app: AppHandle,
        secret: String,
    ) -> Result<SyncStatus, CommandError> {
        blocking(move || {
            let state = app.state::<AppState>();
            super::sync_edit_secret(&state, SecretString::from(secret))
        })
        .await?
    }

    #[tauri::command(async)]
    pub fn sync_leave(state: State<'_, AppState>) -> Result<SyncStatus, CommandError> {
        super::sync_leave(&state)
    }

    #[tauri::command(async)]
    pub fn sync_dismiss_findings(state: State<'_, AppState>) -> Result<(), CommandError> {
        super::sync_dismiss_findings(&state)
    }
}

#[cfg(test)]
mod tests;
