//! Tests of the update state machine over a fake backend and a fake host.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fernjo_core::{AppSettings, DataDirSource, settings};
use tokio::sync::Notify;

use super::*;
use crate::sync_runner::finish_is_dropped;

type Log = Arc<Mutex<Vec<String>>>;

struct FakeBackend {
    log: Log,
    /// The version the feed announces; `Err` is a feed failure.
    announce: Mutex<Result<Option<&'static str>, FeedError>>,
    download: Mutex<Result<Vec<u8>, FeedError>>,
    gate: Mutex<Option<Arc<Notify>>>,
    on_download: Mutex<Option<Box<dyn Fn() + Send>>>,
    checks: AtomicUsize,
    downloads: AtomicUsize,
    last_skipped: Mutex<Option<Option<String>>>,
}

impl FakeBackend {
    fn new(log: &Log) -> Arc<Self> {
        Arc::new(Self {
            log: Arc::clone(log),
            announce: Mutex::new(Ok(Some("0.4.1"))),
            download: Mutex::new(Ok(vec![1, 2, 3])),
            gate: Mutex::new(None),
            on_download: Mutex::new(None),
            checks: AtomicUsize::new(0),
            downloads: AtomicUsize::new(0),
            last_skipped: Mutex::new(None),
        })
    }
}

fn push(log: &Log, what: &str) {
    log.lock().unwrap().push(what.to_owned());
}

impl UpdateBackend for FakeBackend {
    type Handle = u32;

    async fn check(
        &self,
        current: &str,
        skipped: Option<&str>,
    ) -> Result<Option<Found<u32>>, FeedError> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        *self.last_skipped.lock().unwrap() = Some(skipped.map(str::to_owned));
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let announced = (*self.announce.lock().unwrap())?;
        Ok(announced
            .filter(|v| decide::is_offered(current, v, skipped))
            .map(|v| Found {
                version: v.to_owned(),
                notes: "notes".to_owned(),
                handle: 7,
            }))
    }

    async fn download(&self, _handle: &u32, progress: Progress) -> Result<Vec<u8>, FeedError> {
        self.downloads.fetch_add(1, Ordering::SeqCst);
        push(&self.log, "download");
        progress(3, Some(3));
        if let Some(f) = self.on_download.lock().unwrap().as_ref() {
            f();
        }
        self.download.lock().unwrap().clone()
    }

    fn install(&self, _handle: &u32, bytes: Vec<u8>) -> Result<(), &'static str> {
        assert_eq!(bytes, vec![1, 2, 3]);
        push(&self.log, "install");
        Ok(())
    }
}

struct Inner {
    _dirs: [tempfile::TempDir; 2],
    state: AppState,
    rt: Mutex<UpdaterRuntime<u32>>,
    log: Log,
    events: Mutex<Vec<UpdateStatus>>,
    connected: AtomicUsize,
    view: Mutex<InstallView>,
    confirm: AtomicBool,
    backend: Arc<FakeBackend>,
}

#[derive(Clone)]
struct FakeHost(Arc<Inner>);

impl FakeHost {
    fn new(backend: &Arc<FakeBackend>, log: &Log) -> Self {
        let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
        let mut state = AppState::open(Some(dirs[0].path().to_path_buf()));
        state.config_dir = Some(dirs[1].path().to_path_buf());
        state.source = DataDirSource::Default;
        Self(Arc::new(Inner {
            _dirs: dirs,
            state,
            rt: Mutex::new(UpdaterRuntime::default()),
            log: Arc::clone(log),
            events: Mutex::new(Vec::new()),
            connected: AtomicUsize::new(0),
            view: Mutex::new(InstallView::SelfUpdate),
            confirm: AtomicBool::new(true),
            backend: Arc::clone(backend),
        }))
    }

    fn events(&self) -> Vec<UpdateStatus> {
        self.0.events.lock().unwrap().clone()
    }

    fn settings_path(&self) -> std::path::PathBuf {
        self.0
            .state
            .config_dir
            .clone()
            .unwrap()
            .join(settings::FILE_NAME)
    }
}

impl Host for FakeHost {
    type Handle = u32;

    fn state(&self) -> &AppState {
        &self.0.state
    }
    fn runtime(&self) -> &Mutex<UpdaterRuntime<u32>> {
        &self.0.rt
    }
    fn current_version(&self) -> String {
        "0.4.0".to_owned()
    }
    fn install_view(&self) -> InstallView {
        *self.0.view.lock().unwrap()
    }
    fn emit(&self, status: &UpdateStatus) {
        self.0.events.lock().unwrap().push(status.clone());
    }
    fn connected_count(&self) -> usize {
        self.0.connected.load(Ordering::SeqCst)
    }
    fn prompt_open(&self) -> bool {
        false
    }
    fn ask(&self, connected: usize) {
        assert!(connected > 0);
        push(&self.0.log, "dialog");
        if self.0.confirm.load(Ordering::SeqCst) {
            self.go();
        }
    }
    fn go(&self) {
        push(&self.0.log, "close_all");
        push(&self.0.log, "push");
        finish_install(&*self.0.backend, self);
    }
    fn after_install(&self) {
        push(&self.0.log, "restart");
    }
}

fn setup() -> (Arc<FakeBackend>, FakeHost, Log) {
    let log: Log = Arc::default();
    let backend = FakeBackend::new(&log);
    let host = FakeHost::new(&backend, &log);
    (backend, host, log)
}

fn logged(log: &Log) -> Vec<String> {
    log.lock().unwrap().clone()
}

#[tokio::test]
async fn check_available_sets_state_and_emits_once() {
    let (backend, host, _) = setup();
    let status = run_check(&*backend, &host, false).await;
    assert_eq!(status.state, UpdateState::Available);
    assert_eq!(status.version.as_deref(), Some("0.4.1"));
    assert_eq!(status.notes.as_deref(), Some("notes"));
    assert_eq!(status.install, Some(InstallView::SelfUpdate));
    assert_eq!(status.current_version, "0.4.0");
    assert_eq!(status.error, None);
    assert_eq!(host.events(), vec![status]);
}

#[tokio::test]
async fn a_manual_check_announces_checking_and_reports_up_to_date() {
    let (backend, host, _) = setup();
    *backend.announce.lock().unwrap() = Ok(None);
    let status = run_check(&*backend, &host, true).await;
    assert_eq!(status.state, UpdateState::UpToDate);
    let states: Vec<_> = host.events().iter().map(|e| e.state).collect();
    assert_eq!(states, vec![UpdateState::Checking, UpdateState::UpToDate]);
}

#[tokio::test]
async fn second_check_while_checking_is_dropped() {
    let (backend, host, _) = setup();
    let gate = Arc::new(Notify::new());
    *backend.gate.lock().unwrap() = Some(Arc::clone(&gate));
    let (first, second) = tokio::join!(run_check(&*backend, &host, true), async {
        while status_of(&host).state != UpdateState::Checking {
            tokio::task::yield_now().await;
        }
        let second = run_check(&*backend, &host, true).await;
        gate.notify_one();
        second
    });
    assert_eq!(second.state, UpdateState::Checking);
    assert_eq!(first.state, UpdateState::Available);
    assert_eq!(backend.checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn automatic_check_failure_leaves_state_idle_and_no_error() {
    for failure in [
        FeedError::Unreachable,
        FeedError::BadFeed,
        FeedError::BadSignature,
    ] {
        let (backend, host, _) = setup();
        *backend.announce.lock().unwrap() = Err(failure);
        let status = run_check(&*backend, &host, false).await;
        assert_eq!(status.state, UpdateState::Idle, "{failure:?}");
        assert_eq!(status.error, None);
        assert!(host.events().is_empty(), "{failure:?}");
    }
}

#[tokio::test]
async fn an_automatic_failure_keeps_the_previous_resting_state() {
    let (backend, host, _) = setup();
    run_check(&*backend, &host, false).await;
    *backend.announce.lock().unwrap() = Err(FeedError::Unreachable);
    let status = run_check(&*backend, &host, false).await;
    assert_eq!(status.state, UpdateState::Available);
    assert_eq!(status.version.as_deref(), Some("0.4.1"));
    assert_eq!(host.events().len(), 1);
}

#[tokio::test]
async fn automatic_check_does_nothing_while_the_setting_is_off() {
    let (backend, host, _) = setup();
    *host.0.state.check_for_updates.lock().unwrap() = false;
    let status = run_check(&*backend, &host, false).await;
    assert_eq!(status.state, UpdateState::Idle);
    assert!(!status.check_enabled);
    assert_eq!(backend.checks.load(Ordering::SeqCst), 0);
    // a manual check is always allowed
    let status = run_check(&*backend, &host, true).await;
    assert_eq!(status.state, UpdateState::Available);
}

#[tokio::test]
async fn manual_check_failure_sets_failed_with_code() {
    let (backend, host, _) = setup();
    *backend.announce.lock().unwrap() = Err(FeedError::Unreachable);
    let status = run_check(&*backend, &host, true).await;
    assert_eq!(status.state, UpdateState::Failed);
    assert_eq!(status.error.as_deref(), Some("feed_unreachable"));
    assert_eq!(host.events().last().unwrap(), &status);
    // a later successful check clears the error
    *backend.announce.lock().unwrap() = Ok(Some("0.4.1"));
    let status = run_check(&*backend, &host, true).await;
    assert_eq!((status.state, status.error), (UpdateState::Available, None));
}

#[tokio::test]
async fn install_with_no_connected_sessions_installs_after_download() {
    let (backend, host, log) = setup();
    run_check(&*backend, &host, true).await;
    run_install(&*backend, &host).await;
    assert_eq!(
        logged(&log),
        ["download", "close_all", "push", "install", "restart"]
    );
    assert_eq!(status_of(&host).state, UpdateState::Installing);
    let states: Vec<_> = host.events().iter().map(|e| e.state).collect();
    assert!(states.contains(&UpdateState::Downloading));
    assert!(states.contains(&UpdateState::Ready));
    assert_eq!(states.last(), Some(&UpdateState::Installing));
}

#[tokio::test]
async fn install_with_connected_sessions_waits_for_confirmation_then_closes_sessions_first() {
    let (backend, host, log) = setup();
    host.0.connected.store(2, Ordering::SeqCst);
    run_check(&*backend, &host, true).await;
    run_install(&*backend, &host).await;
    assert_eq!(
        logged(&log),
        [
            "download",
            "dialog",
            "close_all",
            "push",
            "install",
            "restart"
        ]
    );
}

#[tokio::test]
async fn keep_open_keeps_ready_and_second_install_skips_download() {
    let (backend, host, log) = setup();
    host.0.connected.store(1, Ordering::SeqCst);
    host.0.confirm.store(false, Ordering::SeqCst);
    run_check(&*backend, &host, true).await;
    let status = run_install(&*backend, &host).await;
    assert_eq!(status.state, UpdateState::Ready);
    assert!(host.0.rt.lock().unwrap().bytes.is_some());
    assert_eq!(logged(&log), ["download", "dialog"]);

    host.0.confirm.store(true, Ordering::SeqCst);
    run_install(&*backend, &host).await;
    assert_eq!(backend.downloads.load(Ordering::SeqCst), 1);
    assert_eq!(
        logged(&log),
        [
            "download",
            "dialog",
            "dialog",
            "close_all",
            "push",
            "install",
            "restart"
        ]
    );
}

#[tokio::test]
async fn connected_count_is_read_at_install_time() {
    let (backend, host, log) = setup();
    run_check(&*backend, &host, true).await;
    // a session connects while the download runs
    let h = host.clone();
    *backend.on_download.lock().unwrap() =
        Some(Box::new(move || h.0.connected.store(1, Ordering::SeqCst)));
    host.0.confirm.store(false, Ordering::SeqCst);
    let status = run_install(&*backend, &host).await;
    assert_eq!(status.state, UpdateState::Ready);
    assert_eq!(logged(&log), ["download", "dialog"]);
}

#[tokio::test]
async fn oversize_download_sets_failed_download_refused() {
    let (backend, host, log) = setup();
    run_check(&*backend, &host, true).await;
    *backend.download.lock().unwrap() = Err(FeedError::DownloadRefused("too_large"));
    let status = run_install(&*backend, &host).await;
    assert_eq!(status.state, UpdateState::Failed);
    assert_eq!(status.error.as_deref(), Some("download_refused"));
    assert!(host.0.rt.lock().unwrap().bytes.is_none());
    assert_eq!(logged(&log), ["download"]);
    // the version stays on offer and a retry downloads again
    assert_eq!(status.version.as_deref(), Some("0.4.1"));
    *backend.download.lock().unwrap() = Ok(vec![1, 2, 3]);
    run_install(&*backend, &host).await;
    assert_eq!(backend.downloads.load(Ordering::SeqCst), 2);
    assert_eq!(logged(&log).last().map(String::as_str), Some("restart"));
}

#[tokio::test]
async fn a_refused_download_leaves_nothing_to_install() {
    for refusal in [
        FeedError::BadSignature,
        FeedError::DownloadRefused("too_large"),
        FeedError::DownloadRefused("failed"),
        FeedError::DownloadRefused("stalled"),
    ] {
        let (backend, host, log) = setup();
        host.0.connected.store(1, Ordering::SeqCst);
        run_check(&*backend, &host, true).await;
        *backend.download.lock().unwrap() = Err(refusal);
        let status = run_install(&*backend, &host).await;
        assert_eq!(status.state, UpdateState::Failed, "{refusal:?}");
        assert!(host.0.rt.lock().unwrap().bytes.is_none());
        finish_install(&*backend, &host);
        // no dialog, no ended sessions, no install
        assert_eq!(logged(&log), ["download"], "{refusal:?}");
    }
}

/// Every non-comment line of the app's sources (test files aside) that holds `needle`.
fn source_lines_with(needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|e| e == "rs")
                && path.file_name().is_some_and(|n| n != "tests.rs")
            {
                let source = std::fs::read_to_string(&path).unwrap();
                found.extend(
                    source
                        .lines()
                        .map(str::trim)
                        .filter(|l| !l.starts_with("//") && l.contains(needle))
                        .map(str::to_owned),
                );
            }
        }
    }
    found
}

/// The plugin's own hook hides every window and drops the single-instance lock before it starts
/// the Windows setup, so a start that fails would leave fernjo running unseen (M14 review #82).
/// The hook cannot be observed from outside the plugin, so the source is pinned: one place builds
/// an updater, and it replaces the hook there.
#[test]
fn the_plugin_exit_hook_is_replaced_so_a_failed_setup_start_keeps_the_app_usable() {
    assert_eq!(
        source_lines_with("updater_builder("),
        ["let builder = app.updater_builder().on_before_exit(|| {});"]
    );
    assert_eq!(source_lines_with(".updater()"), Vec::<String>::new());
}

#[tokio::test]
async fn a_confirmation_that_no_longer_applies_ends_nothing() {
    let (backend, host, _) = setup();
    run_check(&*backend, &host, true).await;
    // the download the dialog asked about was replaced: no session may end for it
    assert!(!commit_confirmed(CloseThen::Install, &host.0.rt));
    assert!(commit_confirmed(CloseThen::Exit, &host.0.rt));
    assert!(commit_confirmed(CloseThen::Restart, &host.0.rt));
    // the close dialog asks this before it ends the sessions
    let lib = include_str!("../lib.rs");
    let dialog = &lib[lib.find("pub(crate) fn ask_to_close").unwrap()..];
    let asked = dialog
        .find("updater::commit_confirmed(then,")
        .expect("asked");
    assert!(asked < dialog.find("sessions.close_all()").unwrap());
}

#[tokio::test]
async fn a_check_that_ends_after_the_confirmation_keeps_the_committed_download() {
    let (backend, host, log) = setup();
    host.0.connected.store(1, Ordering::SeqCst);
    host.0.confirm.store(false, Ordering::SeqCst);
    run_check(&*backend, &host, true).await;
    run_install(&*backend, &host).await;
    assert!(commit_confirmed(CloseThen::Install, &host.0.rt));
    // the sessions end and the last push runs; meanwhile a check finds yet another version
    *backend.announce.lock().unwrap() = Ok(Some("0.4.2"));
    run_check(&*backend, &host, true).await;
    finish_install(&*backend, &host);
    assert!(
        logged(&log).contains(&"install".to_owned()),
        "{:?}",
        logged(&log)
    );
}

#[tokio::test]
async fn release_page_only_install_never_downloads() {
    let (backend, host, log) = setup();
    *host.0.view.lock().unwrap() = InstallView::ReleasePage {
        reason: "package_install",
    };
    let checked = run_check(&*backend, &host, true).await;
    assert_eq!(
        checked.install,
        Some(InstallView::ReleasePage {
            reason: "package_install"
        })
    );
    let status = run_install(&*backend, &host).await;
    assert_eq!(status, checked);
    assert_eq!(backend.downloads.load(Ordering::SeqCst), 0);
    assert!(logged(&log).is_empty());
}

#[tokio::test]
async fn install_while_not_available_does_nothing() {
    let (backend, host, log) = setup();
    let status = run_install(&*backend, &host).await;
    assert_eq!(status.state, UpdateState::Idle);
    assert!(logged(&log).is_empty());
    assert!(host.events().is_empty());
}

#[tokio::test]
async fn an_install_that_fails_sets_failed_and_frees_the_bytes() {
    struct Failing(Arc<FakeBackend>);
    impl UpdateBackend for Failing {
        type Handle = u32;
        fn check(
            &self,
            c: &str,
            s: Option<&str>,
        ) -> impl Future<Output = Result<Option<Found<u32>>, FeedError>> + Send {
            self.0.check(c, s)
        }
        fn download(
            &self,
            h: &u32,
            p: Progress,
        ) -> impl Future<Output = Result<Vec<u8>, FeedError>> + Send {
            self.0.download(h, p)
        }
        fn install(&self, _: &u32, _: Vec<u8>) -> Result<(), &'static str> {
            Err("install_failed")
        }
    }
    let (inner, host, _) = setup();
    let backend = Failing(inner);
    run_check(&backend, &host, true).await;
    host.0.rt.lock().unwrap().state = UpdateState::Ready;
    host.0.rt.lock().unwrap().bytes = Some(vec![9]);
    finish_install(&backend, &host);
    let status = status_of(&host);
    assert_eq!(status.state, UpdateState::Failed);
    assert_eq!(status.error.as_deref(), Some("install_failed"));
    assert!(host.0.rt.lock().unwrap().bytes.is_none());
}

#[tokio::test]
async fn skip_stores_version_and_hides_it_from_automatic_checks() {
    let (backend, host, _) = setup();
    run_check(&*backend, &host, true).await;
    let st = host.state();
    let skipped = skip_in(st, host.runtime(), "0.4.0").unwrap();
    assert!(skipped.skipped);
    assert_eq!(
        st.skipped_update_version.lock().unwrap().as_deref(),
        Some("0.4.1")
    );
    let saved = settings::load(&host.settings_path()).unwrap();
    assert_eq!(saved.skipped_update_version.as_deref(), Some("0.4.1"));

    // an automatic check is told the skipped version and offers nothing
    let status = run_check(&*backend, &host, false).await;
    assert_eq!(
        *backend.last_skipped.lock().unwrap(),
        Some(Some("0.4.1".to_owned()))
    );
    assert_eq!(status.state, UpdateState::UpToDate);
    assert_eq!(status.version, None);

    // a manual check shows it again, marked as skipped
    let status = run_check(&*backend, &host, true).await;
    assert_eq!(*backend.last_skipped.lock().unwrap(), Some(None));
    assert_eq!(status.state, UpdateState::Available);
    assert!(status.skipped);

    // a newer version is offered to automatic checks
    *backend.announce.lock().unwrap() = Ok(Some("0.4.2"));
    let status = run_check(&*backend, &host, false).await;
    assert_eq!(status.version.as_deref(), Some("0.4.2"));
    assert!(!status.skipped);
}

#[tokio::test]
async fn skip_refused_when_settings_unusable() {
    let (backend, host, _) = setup();
    // nothing on offer
    let err = skip_in(host.state(), host.runtime(), "0.4.0").unwrap_err();
    assert_eq!(err.code, "no_update");

    run_check(&*backend, &host, true).await;
    let text = r#"{"schemaVersion":9,"dataDir":"/x"}"#;
    std::fs::write(host.settings_path(), text).unwrap();
    let err = skip_in(host.state(), host.runtime(), "0.4.0").unwrap_err();
    assert_eq!(err.code, "settings_unusable");
    assert_eq!(*host.state().skipped_update_version.lock().unwrap(), None);
    assert!(!status_of(&host).skipped);
    assert_eq!(std::fs::read_to_string(host.settings_path()).unwrap(), text);
}

#[tokio::test]
async fn a_check_while_ready_keeps_the_download_unless_another_version_appears() {
    let (backend, host, _) = setup();
    host.0.connected.store(1, Ordering::SeqCst);
    host.0.confirm.store(false, Ordering::SeqCst);
    run_check(&*backend, &host, true).await;
    run_install(&*backend, &host).await;
    assert_eq!(status_of(&host).state, UpdateState::Ready);

    let status = run_check(&*backend, &host, true).await;
    assert_eq!(status.state, UpdateState::Ready);
    assert!(host.0.rt.lock().unwrap().bytes.is_some());

    *backend.announce.lock().unwrap() = Err(FeedError::Unreachable);
    assert_eq!(
        run_check(&*backend, &host, true).await.state,
        UpdateState::Ready
    );

    *backend.announce.lock().unwrap() = Ok(Some("0.4.2"));
    let status = run_check(&*backend, &host, true).await;
    assert_eq!(status.state, UpdateState::Available);
    assert_eq!(status.version.as_deref(), Some("0.4.2"));
    assert!(host.0.rt.lock().unwrap().bytes.is_none());
}

#[test]
fn the_status_serialises_in_camel_case() {
    let rt = UpdaterRuntime::<u32>::default();
    let o = Outside {
        enabled: true,
        skipped: None,
        unsaved: Some(UnsavedReason::Env),
        current: "0.4.0".into(),
    };
    let json = serde_json::to_value(assemble(&rt, &o)).unwrap();
    assert_eq!(json["state"], "idle");
    assert_eq!(json["currentVersion"], "0.4.0");
    assert_eq!(json["checkEnabled"], true);
    assert_eq!(json["unsavedReason"], "env");
    assert_eq!(json["skipped"], false);
    assert!(json["version"].is_null());
    assert_eq!(
        serde_json::to_value(UpdateState::UpToDate).unwrap(),
        "upToDate"
    );
    assert_eq!(
        serde_json::to_value(InstallView::ReleasePage { reason: "x" }).unwrap(),
        serde_json::json!({"kind": "releasePage", "reason": "x"})
    );
    assert_eq!(
        serde_json::to_value(InstallView::SelfUpdate).unwrap(),
        serde_json::json!({"kind": "selfUpdate"})
    );
}

#[test]
fn the_release_page_is_the_tag_page_of_the_offer_or_the_generic_page() {
    let rt = Mutex::new(UpdaterRuntime::<u32>::default());
    let latest = "https://github.com/benjo-code/fernjo/releases/latest";
    assert_eq!(decide::RELEASES_LATEST_URL, latest);
    assert_eq!(release_page_of(&rt), latest);
    let offer = |version: &str| Available {
        version: version.into(),
        notes: String::new(),
        install: InstallView::SelfUpdate,
        handle: Arc::new(1u32),
    };
    lock(&rt).available = Some(offer("0.4.1"));
    assert_eq!(
        release_page_of(&rt),
        "https://github.com/benjo-code/fernjo/releases/tag/v0.4.1"
    );
    lock(&rt).available = Some(offer("0.4.1-rc.1"));
    assert_eq!(release_page_of(&rt), latest);
    // the URL passes the same check the opener path applies
    assert!(crate::sessions::check_external_url(latest).is_ok());
}

#[test]
fn set_check_for_updates_returns_the_status_and_keeps_the_version_state() {
    let log: Log = Arc::default();
    let host = FakeHost::new(&FakeBackend::new(&log), &log);
    let st = host.state();
    assert!(update_status(st).check_enabled);
    let status = set_check_for_updates(st, false).unwrap();
    assert!(!status.check_enabled);
    assert_eq!(status.state, UpdateState::Idle);
    assert_eq!(status.current_version, CURRENT_VERSION);
    let saved: AppSettings = settings::load(&host.settings_path()).unwrap();
    assert_eq!(saved.check_for_updates, Some(false));
}

#[test]
fn the_install_view_texts_are_fixed() {
    assert_eq!(view_of(InstallKind::SelfUpdate), InstallView::SelfUpdate);
    assert_eq!(
        view_of(InstallKind::ReleasePageOnly(
            ReleasePageReason::PackageInstall
        )),
        InstallView::ReleasePage {
            reason: "package_install"
        }
    );
    assert_eq!(
        view_of(InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable)),
        InstallView::ReleasePage {
            reason: "not_writable"
        }
    );
}

#[test]
fn the_write_probe_leaves_nothing_behind() {
    let dir = tempfile::tempdir().unwrap();
    assert!(dir_is_writable(dir.path()));
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    assert!(!dir_is_writable(&dir.path().join("missing")));
}

#[tokio::test]
async fn a_returning_install_clears_sync_closing_so_the_window_closes_and_a_retry_runs() {
    use std::sync::atomic::Ordering::SeqCst;
    struct Failing(Arc<FakeBackend>, AtomicBool);
    impl UpdateBackend for Failing {
        type Handle = u32;
        fn check(
            &self,
            c: &str,
            s: Option<&str>,
        ) -> impl Future<Output = Result<Option<Found<u32>>, FeedError>> + Send {
            self.0.check(c, s)
        }
        fn download(
            &self,
            h: &u32,
            p: Progress,
        ) -> impl Future<Output = Result<Vec<u8>, FeedError>> + Send {
            self.0.download(h, p)
        }
        fn install(&self, h: &u32, b: Vec<u8>) -> Result<(), &'static str> {
            if self.1.load(SeqCst) {
                self.0.install(h, b)
            } else {
                Err("install_failed")
            }
        }
    }
    let (inner, host, log) = setup();
    let backend = Failing(inner, AtomicBool::new(false));
    let closing = &host.state().sync_closing;
    run_check(&backend, &host, true).await;
    {
        let mut g = host.0.rt.lock().unwrap();
        g.state = UpdateState::Ready;
        g.bytes = Some(vec![1, 2, 3]);
    }

    // `finish_after_push` has set the flag; the install fails and returns
    closing.store(true, SeqCst);
    finish_install(&backend, &host);
    assert_eq!(status_of(&host).state, UpdateState::Failed);
    assert!(!closing.load(SeqCst));
    assert!(!crate::sync_runner::hold_close(host.state()));

    // the state is no longer `Ready`: nothing is installed, the flag is cleared as well
    closing.store(true, SeqCst);
    finish_install(&backend, &host);
    assert!(!closing.load(SeqCst));
    assert!(!logged(&log).contains(&"install".to_owned()));

    // a retry (`Failed` -> download -> `Ready` -> `go`) installs and returns the flag cleared
    backend.1.store(true, SeqCst);
    closing.store(true, SeqCst);
    run_install(&backend, &host).await;
    assert!(logged(&log).contains(&"install".to_owned()));
    assert!(!closing.load(SeqCst));
}

/// A backend whose chosen step panics, to prove the run slot is given back.
struct Panicking {
    inner: Arc<FakeBackend>,
    on: &'static str,
}

impl UpdateBackend for Panicking {
    type Handle = u32;
    fn check(
        &self,
        c: &str,
        s: Option<&str>,
    ) -> impl Future<Output = Result<Option<Found<u32>>, FeedError>> + Send {
        let panic_here = self.on == "check";
        let inner = Arc::clone(&self.inner);
        let (c, s) = (c.to_owned(), s.map(str::to_owned));
        async move {
            assert!(!panic_here, "backend panic in check");
            inner.check(&c, s.as_deref()).await
        }
    }
    fn download(
        &self,
        h: &u32,
        p: Progress,
    ) -> impl Future<Output = Result<Vec<u8>, FeedError>> + Send {
        let panic_here = self.on == "download";
        let inner = Arc::clone(&self.inner);
        let h = *h;
        async move {
            assert!(!panic_here, "backend panic in download");
            inner.download(&h, p).await
        }
    }
    fn install(&self, h: &u32, b: Vec<u8>) -> Result<(), &'static str> {
        assert!(self.on != "install", "backend panic in install");
        self.inner.install(h, b)
    }
}

#[tokio::test]
async fn a_panic_in_a_check_gives_the_run_slot_back() {
    let (inner, host, _) = setup();
    let backend = Arc::new(Panicking {
        inner: Arc::clone(&inner),
        on: "check",
    });
    let (b, h) = (Arc::clone(&backend), host.clone());
    let joined = tokio::spawn(async move { run_check(&*b, &h, true).await }).await;
    assert!(joined.unwrap_err().is_panic());
    assert_eq!(status_of(&host).state, UpdateState::Idle);
    // the next check is not dropped as "already checking"
    let status = run_check(&*inner, &host, true).await;
    assert_eq!(status.state, UpdateState::Available);
    assert_eq!(inner.checks.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_panic_in_a_check_from_ready_keeps_the_download_and_the_slot_free() {
    let (inner, host, _) = setup();
    host.0.connected.store(1, Ordering::SeqCst);
    host.0.confirm.store(false, Ordering::SeqCst);
    run_check(&*inner, &host, true).await;
    run_install(&*inner, &host).await;
    assert_eq!(status_of(&host).state, UpdateState::Ready);
    let backend = Arc::new(Panicking {
        inner: Arc::clone(&inner),
        on: "check",
    });
    let (b, h) = (Arc::clone(&backend), host.clone());
    let joined = tokio::spawn(async move { run_check(&*b, &h, true).await }).await;
    assert!(joined.unwrap_err().is_panic());
    assert_eq!(status_of(&host).state, UpdateState::Ready);
    assert!(host.0.rt.lock().unwrap().bytes.is_some());
    assert_eq!(
        run_check(&*inner, &host, true).await.state,
        UpdateState::Ready
    );
    assert_eq!(inner.checks.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_panic_in_a_download_returns_to_available_and_a_retry_runs() {
    let (inner, host, log) = setup();
    run_check(&*inner, &host, true).await;
    let backend = Arc::new(Panicking {
        inner: Arc::clone(&inner),
        on: "download",
    });
    let (b, h) = (Arc::clone(&backend), host.clone());
    let joined = tokio::spawn(async move { run_install(&*b, &h).await }).await;
    assert!(joined.unwrap_err().is_panic());
    let status = status_of(&host);
    assert_eq!(status.state, UpdateState::Available);
    assert_eq!((status.received, status.total), (None, None));
    run_install(&*inner, &host).await;
    assert_eq!(logged(&log).last().map(String::as_str), Some("restart"));
}

#[tokio::test]
async fn a_panic_in_the_install_ends_in_failed_not_installing() {
    let (inner, host, _) = setup();
    let backend = Panicking {
        inner: Arc::clone(&inner),
        on: "install",
    };
    run_check(&backend, &host, true).await;
    {
        let mut g = host.0.rt.lock().unwrap();
        g.state = UpdateState::Ready;
        g.bytes = Some(vec![1, 2, 3]);
    }
    // `finish_after_push` sets the flag before it gets here
    host.state().sync_closing.store(true, Ordering::SeqCst);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        finish_install(&backend, &host);
    }));
    assert!(caught.is_err());
    let status = status_of(&host);
    assert_eq!(status.state, UpdateState::Failed);
    assert_eq!(status.error.as_deref(), Some("install_failed"));
    assert!(!install_blocks_exit(
        status.state == UpdateState::Installing,
        None
    ));
    // a flag left set would make every later close wait for a push that never ends
    assert!(!host.state().sync_closing.load(Ordering::SeqCst));
}

#[tokio::test]
async fn a_confirm_during_a_check_from_ready_still_installs_and_the_check_keeps_the_verdict() {
    let (backend, host, log) = setup();
    host.0.connected.store(1, Ordering::SeqCst);
    host.0.confirm.store(false, Ordering::SeqCst);
    run_check(&*backend, &host, true).await;
    run_install(&*backend, &host).await;
    assert_eq!(status_of(&host).state, UpdateState::Ready);

    let gate = Arc::new(Notify::new());
    *backend.gate.lock().unwrap() = Some(Arc::clone(&gate));
    let (checked, ()) = tokio::join!(run_check(&*backend, &host, true), async {
        while backend.checks.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        // the held download stays installable while the check is out
        assert_eq!(status_of(&host).state, UpdateState::Ready);
        // a second check is still dropped by the run slot
        let second = run_check(&*backend, &host, true).await;
        assert_eq!(second.state, UpdateState::Ready);
        assert_eq!(backend.checks.load(Ordering::SeqCst), 2);
        finish_install(&*backend, &host);
        gate.notify_one();
    });
    assert!(logged(&log).contains(&"install".to_owned()));
    // the late answer of the check must not turn `Installing` back into `Ready`
    assert_eq!(checked.state, UpdateState::Installing);
    assert_eq!(status_of(&host).state, UpdateState::Installing);
    assert!(host.0.rt.lock().unwrap().bytes.is_none());
}

#[test]
fn an_install_in_progress_blocks_a_close_and_an_exit_without_a_code_only() {
    // closing the window and Cmd+Q (no exit code) wait while the files are replaced
    assert!(install_blocks_exit(true, None));
    // the restart that follows a finished install carries an exit code and always passes
    assert!(!install_blocks_exit(true, Some(0)));
    assert!(!install_blocks_exit(true, Some(1)));
    // nothing installing: the usual close and exit rules apply
    assert!(!install_blocks_exit(false, None));
    assert!(!install_blocks_exit(false, Some(0)));
}

#[test]
fn nothing_closes_or_restarts_the_app_while_an_install_runs() {
    let log: Log = Arc::default();
    let host = FakeHost::new(&FakeBackend::new(&log), &log);
    let state = host.state();
    let set = |next| state.updater.lock().unwrap().state = next;
    assert!(!finish_is_dropped(state));
    // the install itself starts from `Ready` through the same path and has to go ahead
    set(UpdateState::Ready);
    assert!(!finish_is_dropped(state));
    // "Restart now" or a confirmed close dialog would cut the replacement of the files
    set(UpdateState::Installing);
    assert!(finish_is_dropped(state));
    // a failed install leaves the window closable
    set(UpdateState::Failed);
    assert!(!finish_is_dropped(state));
    // a push that is already under way ends the window itself
    state.sync_closing.store(true, Ordering::SeqCst);
    assert!(finish_is_dropped(state));
}

#[test]
fn only_the_installing_state_counts_as_an_install_in_progress() {
    let log: Log = Arc::default();
    let host = FakeHost::new(&FakeBackend::new(&log), &log);
    assert!(!installing(&host.0.rt));
    host.0.rt.lock().unwrap().state = UpdateState::Ready;
    assert!(!installing(&host.0.rt));
    host.0.rt.lock().unwrap().state = UpdateState::Installing;
    assert!(installing(&host.0.rt));
}
