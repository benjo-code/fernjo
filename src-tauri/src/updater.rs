//! Automatic updates (M13). Pure decisions live in `decide`, the feed client in `feed`; this file
//! is the state machine, the commands and the install flow.
//!
//! Wire contract (the UI reads it in task 7): every change emits the event `update-status` with
//! the whole [`UpdateStatus`] (camelCase). `state` is one of `idle`, `checking`, `upToDate`,
//! `available`, `downloading`, `ready`, `installing`, `failed`; `install` is
//! `{"kind":"selfUpdate"}` or `{"kind":"releasePage","reason":"package_install"|"not_writable"}`
//! and present while a version is known; `received`/`total` only while downloading (`total`
//! only when the server declared a length); `error` only in `failed`, a fixed code
//! (`feed_unreachable`, `feed_invalid`, `download_refused`, `signature_invalid`,
//! `install_failed`).
//!
//! Rules:
//! - One run slot: a check while checking, downloading or installing is dropped (the current
//!   status comes back), and so is an install while downloading or installing.
//! - An automatic check (`manual == false`) swallows every failure: the state returns to what it
//!   was, no error, no event. It also does not announce "checking" and only emits when the status
//!   changed. A manual check reports its failure as `failed` with the error code and finding
//!   nothing as `upToDate`.
//! - Skipping: `update_skip` stores the available version; automatic checks pass it to the feed
//!   filter, so it is never offered again. A manual check ignores the skipped version (it is
//!   offered again) and `skipped` in the status says that it is the skipped one.
//! - Install order: click, download (the app stays usable), `ready`, then `connected_count` is
//!   read at that moment; nothing connected installs at once, otherwise the dialog asks
//!   (`CloseThen::Install`). "Keep open" leaves `ready` with the bytes held; a second
//!   `update_install` skips the download and asks again. Confirmed: sessions closed, last sync
//!   push, `install`. The plugin's install on Windows hands over to the setup and exits the
//!   process; macOS and Linux replace the files and return, then a restart is requested
//!   (`request_restart`, never `AppHandle::restart`). `download_and_install` is never used.
//! - A page-only install (`InstallView::ReleasePage`) never downloads: `update_install` returns
//!   the status unchanged.
//! - The runtime lock is a leaf: nothing else is locked while it is held, and it is never held
//!   across network I/O, a dialog or the install.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::commands::CommandError;
use crate::session_commands::{UnsavedReason, unsaved_reason};
use crate::sessions::{CloseThen, ExitAction, exit_action};
use crate::state::AppState;
use decide::{InstallEnv, InstallKind, Os, ReleasePageReason};
use feed::{FeedError, FeedUpdate};

pub(crate) mod decide;
pub mod feed;
#[cfg(test)]
mod tests;

/// The installed version (`CARGO_PKG_VERSION`, the workspace version).
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Download progress events are sent at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Ready,
    Installing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstallView {
    SelfUpdate,
    ReleasePage { reason: &'static str },
}

/// What the UI sees (also the payload of the `update-status` event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub state: UpdateState,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub install: Option<InstallView>,
    pub received: Option<u64>,
    pub total: Option<u64>,
    pub error: Option<String>,
    pub check_enabled: bool,
    pub skipped: bool,
    pub unsaved_reason: Option<UnsavedReason>,
}

/// The update the feed announced, as the state machine keeps it.
pub struct Available<H> {
    version: String,
    notes: String,
    install: InstallView,
    handle: Arc<H>,
}

/// The state machine's data. `H` is the backend's update handle (`FeedUpdate` in the app).
pub struct UpdaterRuntime<H = FeedUpdate> {
    state: UpdateState,
    available: Option<Available<H>>,
    received: Option<u64>,
    total: Option<u64>,
    error: Option<String>,
    /// The verified download, held only in `Ready` and `Installing`.
    bytes: Option<Vec<u8>>,
}

impl<H> Default for UpdaterRuntime<H> {
    fn default() -> Self {
        Self {
            state: UpdateState::Idle,
            available: None,
            received: None,
            total: None,
            error: None,
            bytes: None,
        }
    }
}

/// An update the backend found.
pub struct Found<H> {
    pub version: String,
    pub notes: String,
    pub handle: H,
}

/// Progress callback of a download: the size of a chunk and the declared total length.
pub type Progress = Box<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// The network side. The app implements it with `feed` and the plugin; the tests use a fake.
pub trait UpdateBackend: Send + Sync + 'static {
    type Handle: Send + Sync + 'static;

    fn check(
        &self,
        current: &str,
        skipped: Option<&str>,
    ) -> impl Future<Output = Result<Option<Found<Self::Handle>>, FeedError>> + Send;

    fn download(
        &self,
        handle: &Self::Handle,
        progress: Progress,
    ) -> impl Future<Output = Result<Vec<u8>, FeedError>> + Send;

    /// Installs the verified bytes. On Windows a success never returns (the process exits).
    ///
    /// # Errors
    /// A fixed error code.
    fn install(&self, handle: &Self::Handle, bytes: Vec<u8>) -> Result<(), &'static str>;
}

/// Everything around the machine that is not the network: state, clock, sessions, the dialog.
pub trait Host: Clone + Send + Sync + 'static {
    type Handle: Send + Sync + 'static;

    fn state(&self) -> &AppState;
    fn runtime(&self) -> &Mutex<UpdaterRuntime<Self::Handle>>;
    fn current_version(&self) -> String;
    /// Whether this install can update itself (probes the file system; called once per check
    /// that found an update).
    fn install_view(&self) -> InstallView;
    fn emit(&self, status: &UpdateStatus);
    fn connected_count(&self) -> usize;
    fn prompt_open(&self) -> bool;
    /// Shows the "Install update?" dialog; on "Install" it does what [`Host::go`] does.
    fn ask(&self, connected: usize);
    /// Ends the sessions, runs the last sync push and then [`finish_install`].
    fn go(&self);
    /// After a successful install that returned (macOS, Linux).
    fn after_install(&self);
}

fn lock<H>(m: &Mutex<UpdaterRuntime<H>>) -> MutexGuard<'_, UpdaterRuntime<H>> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// What the status needs from outside the runtime; read before the runtime lock is taken.
struct Outside {
    enabled: bool,
    skipped: Option<String>,
    unsaved: Option<UnsavedReason>,
    current: String,
}

fn outside(state: &AppState, current: &str) -> Outside {
    Outside {
        enabled: state.check_for_updates.lock().ok().is_none_or(|g| *g),
        skipped: state
            .skipped_update_version
            .lock()
            .ok()
            .and_then(|g| g.clone()),
        unsaved: unsaved_reason(state),
        current: current.to_owned(),
    }
}

fn assemble<H>(rt: &UpdaterRuntime<H>, o: &Outside) -> UpdateStatus {
    let available = rt.available.as_ref();
    UpdateStatus {
        state: rt.state,
        current_version: o.current.clone(),
        version: available.map(|a| a.version.clone()),
        notes: available.map(|a| a.notes.clone()),
        install: available.map(|a| a.install),
        received: rt.received,
        total: rt.total,
        error: rt.error.clone(),
        check_enabled: o.enabled,
        skipped: available.is_some_and(|a| o.skipped.as_deref() == Some(a.version.as_str())),
        unsaved_reason: o.unsaved,
    }
}

fn snapshot<H>(state: &AppState, rt: &Mutex<UpdaterRuntime<H>>, current: &str) -> UpdateStatus {
    let o = outside(state, current);
    assemble(&lock(rt), &o)
}

/// Changes the runtime under its lock and returns the new status.
fn change<Ho: Host>(host: &Ho, f: impl FnOnce(&mut UpdaterRuntime<Ho::Handle>)) -> UpdateStatus {
    let o = outside(host.state(), &host.current_version());
    let mut g = lock(host.runtime());
    f(&mut g);
    assemble(&g, &o)
}

fn change_emit<Ho: Host>(
    host: &Ho,
    f: impl FnOnce(&mut UpdaterRuntime<Ho::Handle>),
) -> UpdateStatus {
    let status = change(host, f);
    host.emit(&status);
    status
}

fn status_of<Ho: Host>(host: &Ho) -> UpdateStatus {
    snapshot(host.state(), host.runtime(), &host.current_version())
}

fn error(code: &str, message: &str) -> CommandError {
    CommandError {
        code: code.to_string(),
        message: message.to_string(),
    }
}

fn no_update() -> CommandError {
    error("no_update", "there is no update available")
}

fn is_busy(s: UpdateState) -> bool {
    matches!(
        s,
        UpdateState::Checking | UpdateState::Downloading | UpdateState::Installing
    )
}

/// One check. See the module comment for the manual and automatic rules.
pub async fn run_check<B, Ho>(backend: &B, host: &Ho, manual: bool) -> UpdateStatus
where
    B: UpdateBackend,
    Ho: Host<Handle = B::Handle>,
{
    let current = host.current_version();
    let o = outside(host.state(), &current);
    let before;
    let prev;
    {
        let mut g = lock(host.runtime());
        if is_busy(g.state) || (!manual && !o.enabled) {
            return assemble(&g, &o);
        }
        before = assemble(&g, &o);
        prev = (g.state, g.error.take());
        g.state = UpdateState::Checking;
    }
    if manual {
        host.emit(&status_of(host));
    }
    let skipped = if manual { None } else { o.skipped.as_deref() };
    let result = backend.check(&current, skipped).await;
    let view = match &result {
        Ok(Some(_)) => Some(host.install_view()),
        _ => None,
    };
    let after = change(host, |g| {
        let ready = prev.0 == UpdateState::Ready;
        match result {
            Ok(Some(f)) => {
                let same = g.available.as_ref().is_some_and(|a| a.version == f.version);
                if ready && same {
                    // the held download is still the one on offer
                    g.state = prev.0;
                    g.error = prev.1;
                } else {
                    g.available = Some(Available {
                        version: f.version,
                        notes: f.notes,
                        install: view.unwrap_or(InstallView::SelfUpdate),
                        handle: Arc::new(f.handle),
                    });
                    g.state = UpdateState::Available;
                    g.bytes = None;
                    g.received = None;
                    g.total = None;
                    g.error = None;
                }
            }
            Ok(None) if !ready => {
                g.available = None;
                g.state = UpdateState::UpToDate;
                g.error = None;
            }
            Err(e) if manual && !ready => {
                g.state = UpdateState::Failed;
                g.error = Some(e.code().to_owned());
            }
            // an automatic failure, or anything that must not discard a held download
            _ => {
                g.state = prev.0;
                g.error = prev.1;
            }
        }
    });
    if manual || after != before {
        host.emit(&after);
    }
    after
}

/// The first half of the install: download if needed, then decide. Returns the status after it;
/// the install itself follows in [`finish_install`], called through `Host::go` or the dialog.
pub async fn run_install<B, Ho>(backend: &B, host: &Ho) -> UpdateStatus
where
    B: UpdateBackend,
    Ho: Host<Handle = B::Handle>,
{
    let handle = {
        let current = host.current_version();
        let o = outside(host.state(), &current);
        let mut g = lock(host.runtime());
        match g.state {
            UpdateState::Ready => None,
            UpdateState::Available | UpdateState::Failed if g.available.is_some() => {
                let Some(available) = g.available.as_ref() else {
                    return assemble(&g, &o);
                };
                if matches!(available.install, InstallView::ReleasePage { .. }) {
                    return assemble(&g, &o);
                }
                let handle = Arc::clone(&available.handle);
                g.state = UpdateState::Downloading;
                g.received = Some(0);
                g.total = None;
                g.error = None;
                Some(handle)
            }
            _ => return assemble(&g, &o),
        }
    };
    if let Some(handle) = handle {
        host.emit(&status_of(host));
        let progress = progress_sink(host.clone());
        match backend.download(&handle, progress).await {
            Ok(bytes) => {
                change_emit(host, |g| {
                    g.state = UpdateState::Ready;
                    g.bytes = Some(bytes);
                    g.received = None;
                    g.total = None;
                });
            }
            Err(e) => {
                return change_emit(host, |g| {
                    g.state = UpdateState::Failed;
                    g.bytes = None;
                    g.received = None;
                    g.total = None;
                    g.error = Some(e.code().to_owned());
                });
            }
        }
    }
    // read now, not when the click happened: sessions may have connected during the download
    let connected = host.connected_count();
    match exit_action(None, connected, host.prompt_open()) {
        ExitAction::Allow => host.go(),
        ExitAction::Ask => host.ask(connected),
        ExitAction::Prevent => {}
    }
    status_of(host)
}

/// A progress callback that keeps the runtime current and emits at most every `PROGRESS_EVERY`.
fn progress_sink<Ho: Host>(host: Ho) -> Progress {
    let last = Mutex::new(None::<Instant>);
    Box::new(move |chunk, declared| {
        {
            let mut g = lock(host.runtime());
            if g.state != UpdateState::Downloading {
                return;
            }
            g.received = Some(g.received.unwrap_or(0).saturating_add(chunk));
            g.total = declared.or(g.total);
        }
        let due = last.lock().is_ok_and(|mut l| {
            let due = l.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
            if due {
                *l = Some(Instant::now());
            }
            due
        });
        if due {
            host.emit(&status_of(&host));
        }
    })
}

/// The second half: installs the held bytes. Does nothing unless the state is `Ready`.
///
/// `finish_after_push` sets `sync_closing` before it gets here and nothing else resets it, because
/// an exit or a restart ends the process. An install is the one path that can return (it failed,
/// the state was no longer `Ready`, or macOS and Linux replaced the files and only request a
/// restart), so every return clears the flag: the window stays closable and a retry runs again.
/// On Windows a successful install never returns, which is fine.
pub fn finish_install<B, Ho>(backend: &B, host: &Ho)
where
    B: UpdateBackend,
    Ho: Host<Handle = B::Handle>,
{
    install_held(backend, host);
    host.state()
        .sync_closing
        .store(false, std::sync::atomic::Ordering::SeqCst);
}

fn install_held<B, Ho>(backend: &B, host: &Ho)
where
    B: UpdateBackend,
    Ho: Host<Handle = B::Handle>,
{
    let taken = {
        let mut g = lock(host.runtime());
        match (g.state, g.available.as_ref().map(|a| Arc::clone(&a.handle))) {
            (UpdateState::Ready, Some(handle)) => g.bytes.take().map(|b| {
                g.state = UpdateState::Installing;
                (handle, b)
            }),
            _ => None,
        }
    };
    let Some((handle, bytes)) = taken else {
        return;
    };
    host.emit(&status_of(host));
    match backend.install(&handle, bytes) {
        Ok(()) => host.after_install(),
        Err(code) => {
            change_emit(host, |g| {
                g.state = UpdateState::Failed;
                g.error = Some(code.to_owned());
            });
        }
    }
}

// ---------------------------------------------------------------------------------------------
// plain commands over `AppState`
// ---------------------------------------------------------------------------------------------

pub fn update_status(state: &AppState) -> UpdateStatus {
    snapshot(state, &state.updater, CURRENT_VERSION)
}

/// Skips the version that is on offer (`no_update` when there is none): it is stored in
/// `settings.json` and automatic checks never offer it again.
///
/// # Errors
/// `no_update`, or the errors of `app_settings::set_skipped_version` (`settings_unusable`, ...).
pub fn update_skip(state: &AppState) -> Result<UpdateStatus, CommandError> {
    skip_in(state, &state.updater, CURRENT_VERSION)
}

fn skip_in<H>(
    state: &AppState,
    rt: &Mutex<UpdaterRuntime<H>>,
    current: &str,
) -> Result<UpdateStatus, CommandError> {
    let version = lock(rt)
        .available
        .as_ref()
        .map(|a| a.version.clone())
        .ok_or_else(no_update)?;
    crate::app_settings::set_skipped_version(state, Some(version))?;
    Ok(snapshot(state, rt, current))
}

/// Turns the background check on or off (saved like the other settings) and returns the status.
///
/// # Errors
/// `settings_unusable` or `settings_write_failed`.
pub fn set_check_for_updates(state: &AppState, on: bool) -> Result<UpdateStatus, CommandError> {
    crate::app_settings::set_check_for_updates(state, on)?;
    Ok(update_status(state))
}

/// The release page of the version on offer, built from the validated version only.
fn release_page_of<H>(rt: &Mutex<UpdaterRuntime<H>>) -> Result<String, CommandError> {
    let version = lock(rt)
        .available
        .as_ref()
        .map(|a| a.version.clone())
        .ok_or_else(no_update)?;
    decide::release_page_url(&version)
        .ok_or_else(|| error("no_release_page", "this version has no release page"))
}

pub fn update_open_release_page(app: &AppHandle) -> Result<(), CommandError> {
    let url = release_page_of(&app.state::<AppState>().updater)?;
    // the same checks and the same opener as `open_external_url`
    let parsed = crate::sessions::check_external_url(&url)?;
    app.opener()
        .open_url(parsed.as_str(), None::<&str>)
        .map_err(|_| error("open_failed", "the link could not be opened"))
}

pub async fn update_check(app: &AppHandle, manual: bool) -> UpdateStatus {
    run_check(&RealBackend(app.clone()), &AppHost(app.clone()), manual).await
}

pub async fn update_install(app: &AppHandle) -> UpdateStatus {
    run_install(&RealBackend(app.clone()), &AppHost(app.clone())).await
}

/// Runs the install of the held download (called by `sync_runner::finish_after_push`, after the
/// sessions ended and the last push ran).
pub(crate) fn install_now(app: &AppHandle) {
    finish_install(&RealBackend(app.clone()), &AppHost(app.clone()));
}

// ---------------------------------------------------------------------------------------------
// the real backend and host
// ---------------------------------------------------------------------------------------------

struct RealBackend(AppHandle);

impl UpdateBackend for RealBackend {
    type Handle = FeedUpdate;

    async fn check(
        &self,
        current: &str,
        skipped: Option<&str>,
    ) -> Result<Option<Found<FeedUpdate>>, FeedError> {
        Ok(feed::check(&self.0, current, skipped, None)
            .await?
            .map(|u| Found {
                version: u.version.clone(),
                notes: u.notes.clone(),
                handle: u,
            }))
    }

    async fn download(
        &self,
        handle: &FeedUpdate,
        progress: Progress,
    ) -> Result<Vec<u8>, FeedError> {
        feed::download(handle, progress).await
    }

    fn install(&self, handle: &FeedUpdate, bytes: Vec<u8>) -> Result<(), &'static str> {
        handle.handle.install(bytes).map_err(|_| "install_failed")
    }
}

#[derive(Clone)]
struct AppHost(AppHandle);

impl AppHost {
    fn window(&self) -> Option<tauri::Window> {
        self.0
            .get_webview_window("main")
            .map(|w| w.as_ref().window())
    }
}

impl Host for AppHost {
    type Handle = FeedUpdate;

    fn state(&self) -> &AppState {
        self.0.state::<AppState>().inner()
    }

    fn runtime(&self) -> &Mutex<UpdaterRuntime<FeedUpdate>> {
        &self.state().updater
    }

    fn current_version(&self) -> String {
        CURRENT_VERSION.to_owned()
    }

    fn install_view(&self) -> InstallView {
        real_install_view()
    }

    fn emit(&self, status: &UpdateStatus) {
        let _ = self.0.emit("update-status", status);
    }

    fn connected_count(&self) -> usize {
        self.state().sessions.connected_count()
    }

    fn prompt_open(&self) -> bool {
        self.state()
            .close_prompt_open
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    fn ask(&self, connected: usize) {
        // without a window there is nothing to ask in: the state stays `Ready`
        if let Some(window) = self.window() {
            crate::ask_to_close(&window, connected, CloseThen::Install);
        }
    }

    fn go(&self) {
        self.state().sessions.close_all();
        match self.window() {
            Some(window) => crate::sync_runner::finish_after_push(window, CloseThen::Install),
            None => install_now(&self.0),
        }
    }

    fn after_install(&self) {
        self.0.request_restart();
    }
}

fn view_of(kind: InstallKind) -> InstallView {
    match kind {
        InstallKind::SelfUpdate => InstallView::SelfUpdate,
        InstallKind::ReleasePageOnly(ReleasePageReason::PackageInstall) => {
            InstallView::ReleasePage {
                reason: "package_install",
            }
        }
        InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable) => InstallView::ReleasePage {
            reason: "not_writable",
        },
    }
}

fn real_install_view() -> InstallView {
    let os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    };
    let env = InstallEnv {
        os,
        appimage: std::env::var_os("APPIMAGE")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from),
        exe: std::env::current_exe().unwrap_or_default(),
    };
    view_of(decide::install_kind(&env, &dir_is_writable))
}

/// Whether a file can be created in `dir`: creates and removes a probe file whose name cannot
/// collide with an existing one (`create_new`, so nothing is ever overwritten).
fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".fernjo-write-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(file) => {
            drop(file);
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub mod handlers {
    use tauri::{AppHandle, State};

    use super::{CommandError, UpdateStatus};
    use crate::state::AppState;

    #[tauri::command]
    pub fn update_status(state: State<'_, AppState>) -> UpdateStatus {
        super::update_status(&state)
    }

    #[tauri::command]
    pub async fn update_check(app: AppHandle, manual: bool) -> UpdateStatus {
        super::update_check(&app, manual).await
    }

    #[tauri::command]
    pub async fn update_install(app: AppHandle) -> UpdateStatus {
        super::update_install(&app).await
    }

    #[tauri::command]
    pub fn update_skip(state: State<'_, AppState>) -> Result<UpdateStatus, CommandError> {
        super::update_skip(&state)
    }

    #[tauri::command]
    pub fn set_check_for_updates(
        state: State<'_, AppState>,
        on: bool,
    ) -> Result<UpdateStatus, CommandError> {
        super::set_check_for_updates(&state, on)
    }

    #[tauri::command]
    pub fn update_open_release_page(app: AppHandle) -> Result<(), CommandError> {
        super::update_open_release_page(&app)
    }
}
