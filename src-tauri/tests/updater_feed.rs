//! The feed client against a local fake feed (plain HTTP, 127.0.0.1) signed with the test-only key
//! (`tests/fixtures/update-test-key`). The plugin runs inside Tauri's mock runtime.

#[path = "support/fake_feed.rs"]
mod fake_feed;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use fake_feed::{FakeFeed, Route};
use fernjo_app_lib::updater::feed::{self, FeedError, FeedUpdate, Limits};
use serde_json::{Value, json};
use tauri::test::MockRuntime;
use url::Url;

fn manifest_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn key_dir() -> PathBuf {
    manifest_dir().join("tests/fixtures/update-test-key")
}

fn test_pubkey() -> String {
    fs::read_to_string(key_dir().join("test-only.pub"))
        .unwrap()
        .trim()
        .to_owned()
}

fn payload() -> Vec<u8> {
    fs::read(key_dir().join("payload.bin")).unwrap()
}

fn payload_signature() -> String {
    fs::read_to_string(key_dir().join("payload.bin.sig"))
        .unwrap()
        .trim()
        .to_owned()
}

fn conf() -> Value {
    serde_json::from_str(&fs::read_to_string(manifest_dir().join("tauri.conf.json")).unwrap())
        .unwrap()
}

/// A mock app with the updater plugin configured like production (endpoint and
/// `requireSignedVersion` from `tauri.conf.json`) but with `pubkey` and the running `version`
/// chosen by the test. `dangerousInsecureTransportProtocol` is only here so that an HTTP fake
/// feed also works in a release-profile test run.
fn app(pubkey: &str, version: &str) -> tauri::App<MockRuntime> {
    let mut updater = conf()["plugins"]["updater"].clone();
    updater["pubkey"] = json!(pubkey);
    updater["dangerousInsecureTransportProtocol"] = json!(true);
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut().plugins.0.insert("updater".into(), updater);
    ctx.package_info_mut().version = version.parse().unwrap();
    tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(ctx)
        .expect("mock app")
}

fn manifest(version: &str, url: &str, signature: &str, notes: &str) -> String {
    let target = tauri_plugin_updater::target().expect("a supported target");
    json!({
        "version": version,
        "notes": notes,
        "pub_date": "2026-11-26T00:00:00Z",
        "platforms": { target: { "signature": signature, "url": url } }
    })
    .to_string()
}

fn url(s: &str) -> Url {
    s.parse().unwrap()
}

/// Feed at `/latest.json` announcing `version`, download at `/dl/payload.bin`.
fn serve(version: &str, signature: &str, notes: &str, body: Vec<u8>) -> FakeFeed {
    FakeFeed::start_with(|addr| {
        let download = format!("http://{addr}/dl/payload.bin");
        vec![
            (
                "/latest.json",
                Route::Json(manifest(version, &download, signature, notes)),
            ),
            ("/dl/payload.bin", Route::Body(body)),
        ]
    })
}

async fn check_with_feed(
    server: &FakeFeed,
    running: &str,
    skipped: Option<&str>,
) -> Result<Option<FeedUpdate>, FeedError> {
    let app = app(&test_pubkey(), running);
    let endpoint = url(&server.url("/latest.json"));
    feed::check(app.handle(), running, skipped, Some(&endpoint)).await
}

async fn offered(server: &FakeFeed, running: &str) -> FeedUpdate {
    check_with_feed(server, running, None)
        .await
        .expect("check")
        .expect("an update is offered")
}

#[tokio::test]
async fn newer_version_is_offered() {
    let server = serve("0.4.1", &payload_signature(), "notes", payload());
    let update = offered(&server, "0.4.0").await;
    assert_eq!(update.version, "0.4.1");
}

#[tokio::test]
async fn same_and_older_version_are_not_offered() {
    for announced in ["0.4.0", "0.3.9"] {
        let server = serve(announced, &payload_signature(), "", payload());
        assert!(
            check_with_feed(&server, "0.4.0", None)
                .await
                .unwrap()
                .is_none(),
            "{announced} must not be offered"
        );
    }
}

#[tokio::test]
async fn pre_release_is_not_offered() {
    // the plugin's own comparator is plain `>` and would offer it; our check filters it
    let server = serve("0.5.0-rc.1", &payload_signature(), "", payload());
    assert!(
        check_with_feed(&server, "0.4.0", None)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn skipped_version_is_not_offered() {
    let server = serve("0.4.1", &payload_signature(), "", payload());
    assert!(
        check_with_feed(&server, "0.4.0", Some("0.4.1"))
            .await
            .unwrap()
            .is_none()
    );
}

/// The fixture is signed for 9.9.9, so these feeds announce 9.9.9 (the signed version is bound).
async fn download_of(
    server: &FakeFeed,
    pubkey: &str,
    limits: Limits,
) -> Result<Vec<u8>, FeedError> {
    let app = app(pubkey, "0.4.0");
    let endpoint = url(&server.url("/latest.json"));
    let update = feed::check(app.handle(), "0.4.0", None, Some(&endpoint))
        .await
        .expect("check")
        .expect("offered");
    feed::download_with(&update, limits, |_, _| {}).await
}

#[tokio::test]
async fn good_download_returns_the_verified_bytes() {
    let server = serve("9.9.9", &payload_signature(), "", payload());
    let bytes = download_of(&server, &test_pubkey(), Limits::production())
        .await
        .expect("download");
    assert_eq!(bytes, payload());
}

#[tokio::test]
async fn missing_signature_is_refused() {
    let server = serve("9.9.9", "", "", payload());
    let err = download_of(&server, &test_pubkey(), Limits::production())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "signature_invalid");
}

#[tokio::test]
async fn wrong_key_signature_is_refused() {
    // the payload is signed by the test key; the app pins the owner's real key
    let real = conf()["plugins"]["updater"]["pubkey"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(real, test_pubkey());
    let server = serve("9.9.9", &payload_signature(), "", payload());
    let err = download_of(&server, &real, Limits::production())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "signature_invalid");
}

#[tokio::test]
async fn tampered_download_is_refused() {
    let mut body = payload();
    body[100] ^= 0x01;
    let server = serve("9.9.9", &payload_signature(), "", body);
    let err = download_of(&server, &test_pubkey(), Limits::production())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "signature_invalid");
}

#[tokio::test]
async fn signed_version_must_match_the_announced_one() {
    // a valid signature for 9.9.9 paired with an announced 10.0.0 (a tampered feed)
    let server = serve("10.0.0", &payload_signature(), "", payload());
    let err = download_of(&server, &test_pubkey(), Limits::production())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "signature_invalid");
}

#[tokio::test]
async fn unreachable_feed_is_feed_unreachable_and_changes_nothing() {
    // a port that was just closed
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let app = app(&test_pubkey(), "0.4.0");
    let endpoint = url(&format!("http://127.0.0.1:{port}/latest.json"));
    let Err(err) = feed::check(app.handle(), "0.4.0", None, Some(&endpoint)).await else {
        panic!("must fail");
    };
    assert_eq!(err.code(), "feed_unreachable");
    // `check` is a pure function of the feed: it keeps no state of its own
}

#[tokio::test]
async fn a_404_and_a_broken_body_are_told_apart() {
    let server = FakeFeed::start(vec![
        ("/missing", Route::Status(404)),
        ("/broken", Route::Json("not json".into())),
        ("/wrong-shape", Route::Json(r#"{"version":"1.0.0"}"#.into())),
    ]);
    let app = app(&test_pubkey(), "0.4.0");
    for (path, code) in [
        ("/missing", "feed_unreachable"),
        ("/broken", "feed_invalid"),
        ("/wrong-shape", "feed_invalid"),
    ] {
        let endpoint = url(&server.url(path));
        let Err(err) = feed::check(app.handle(), "0.4.0", None, Some(&endpoint)).await else {
            panic!("{path} must fail");
        };
        assert_eq!(err.code(), code, "{path}");
    }
}

async fn bad_feed(body: String) -> FeedError {
    let server = FakeFeed::start(vec![("/latest.json", Route::Json(body))]);
    check_with_feed(&server, "0.4.0", None)
        .await
        .expect_err("a feed like this must not be offered")
}

#[tokio::test]
async fn non_semver_feed_version_is_feed_invalid_and_offers_nothing() {
    for version in ["banana", "1.2", ""] {
        let body = manifest(version, "http://127.0.0.1:9/x", &payload_signature(), "");
        assert_eq!(
            bad_feed(body).await,
            FeedError::BadFeed,
            "version {version:?}"
        );
    }
}

#[tokio::test]
async fn empty_platforms_map_is_feed_invalid_and_offers_nothing() {
    let body = json!({ "version": "0.4.1", "platforms": {} }).to_string();
    assert_eq!(bad_feed(body).await, FeedError::BadFeed);
}

#[tokio::test]
async fn platforms_without_this_target_is_feed_invalid() {
    let body = json!({
        "version": "0.4.1",
        "platforms": { "plan9-mips": { "signature": payload_signature(), "url": "http://127.0.0.1:9/x" } }
    })
    .to_string();
    assert_eq!(bad_feed(body).await, FeedError::BadFeed);
}

#[tokio::test]
async fn an_oversized_feed_is_feed_invalid_and_offers_nothing() {
    // a valid manifest padded past the cap: parsing it would cost many times its size
    let mut body: Value = serde_json::from_str(&manifest(
        "0.4.1",
        "http://127.0.0.1:9/x",
        &payload_signature(),
        "",
    ))
    .unwrap();
    body["pad"] = json!("x".repeat(feed::MAX_FEED_BYTES));
    assert_eq!(bad_feed(body.to_string()).await, FeedError::BadFeed);
}

#[tokio::test]
async fn an_endless_feed_is_cut_off_at_the_cap() {
    // no Content-Length: only the received bytes can stop it
    let total = 64 * 1024 * 1024;
    let sent = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let server = FakeFeed::start(vec![(
        "/latest.json",
        Route::Endless {
            total,
            sent: sent.clone(),
            done: done.clone(),
        },
    )]);
    let result = check_with_feed(&server, "0.4.0", None).await;
    assert_eq!(result.err(), Some(FeedError::BadFeed));
    let waited = Instant::now();
    while !done.load(Ordering::SeqCst) && waited.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        sent.load(Ordering::SeqCst) < total / 2,
        "the server kept sending: {} bytes",
        sent.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn a_feed_with_the_longest_release_notes_is_accepted() {
    let notes = "\"".repeat(16 * 1024);
    let server = serve("0.4.1", &payload_signature(), &notes, payload());
    assert_eq!(offered(&server, "0.4.0").await.notes.len(), 16 * 1024);
}

/// An installed v0.4.0 can only ever be updated if the check works against GitHub itself: its TLS
/// and the redirects to the asset host. The fake feed is plain HTTP, so this asks the real host
/// for a release file that exists and is not a manifest.
#[tokio::test]
#[ignore = "talks to github.com: run with --ignored"]
async fn the_real_release_host_is_reached_over_https() {
    let app = app(&test_pubkey(), "0.4.0");
    let endpoint = url("https://github.com/benjo-code/fernjo/releases/latest/download/SHA256SUMS");
    let result = feed::check(app.handle(), "0.4.0", None, Some(&endpoint)).await;
    // fetched and then refused as a manifest; a TLS or network failure would be `Unreachable`
    assert_eq!(result.err(), Some(FeedError::BadFeed));
}

fn limits(cap: u64, stall_ms: u64) -> Limits {
    Limits {
        cap,
        stall: Duration::from_millis(stall_ms),
    }
}

async fn download_from_route(
    route: Route,
    limits: Limits,
) -> (Result<Vec<u8>, FeedError>, FakeFeed) {
    let server = FakeFeed::start_with(|addr| {
        let download = format!("http://{addr}/dl/payload.bin");
        vec![
            (
                "/latest.json",
                Route::Json(manifest("9.9.9", &download, &payload_signature(), "")),
            ),
            ("/dl/payload.bin", route),
        ]
    });
    let result = download_of(&server, &test_pubkey(), limits).await;
    (result, server)
}

#[tokio::test]
async fn oversize_declared_length_is_refused_before_streaming() {
    let started = Instant::now();
    let (result, _server) = download_from_route(
        Route::Stall {
            declared: 1_000_000,
            first: vec![0; 100],
        },
        limits(1024, 10_000),
    )
    .await;
    assert_eq!(result.unwrap_err(), FeedError::DownloadRefused("too_large"));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "refused at the first chunk, not after the stall timeout"
    );
}

#[tokio::test]
async fn stream_past_the_cap_is_cut_off() {
    // no Content-Length: only the received bytes can stop it. At 1 KiB per millisecond the
    // handler would need over a minute for all 64 MiB, so if it ends within seconds the client
    // closed the connection after the cap.
    let total = 64 * 1024 * 1024;
    let sent = Arc::new(AtomicUsize::new(0));
    let done = Arc::new(AtomicBool::new(false));
    let (result, _server) = download_from_route(
        Route::Endless {
            total,
            sent: sent.clone(),
            done: done.clone(),
        },
        limits(4096, 10_000),
    )
    .await;
    assert_eq!(result.unwrap_err(), FeedError::DownloadRefused("too_large"));
    let waited = Instant::now();
    while !done.load(Ordering::SeqCst) && waited.elapsed() < Duration::from_secs(20) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        done.load(Ordering::SeqCst),
        "the server never saw the connection close"
    );
    assert!(
        sent.load(Ordering::SeqCst) < total / 2,
        "the server kept sending: {} bytes",
        sent.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn stalled_download_times_out() {
    let started = Instant::now();
    let (result, _server) = download_from_route(
        Route::Stall {
            declared: 4096,
            first: vec![0; 100],
        },
        limits(1_000_000, 300),
    )
    .await;
    assert_eq!(result.unwrap_err(), FeedError::DownloadRefused("stalled"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn progress_reports_received_bytes_and_declared_length() {
    let server = serve("9.9.9", &payload_signature(), "", payload());
    let update = offered(&server, "0.4.0").await;
    let received = Arc::new(AtomicU64::new(0));
    let declared = Arc::new(AtomicU64::new(0));
    let (r, d) = (received.clone(), declared.clone());
    feed::download_with(&update, Limits::production(), move |n, total| {
        r.fetch_add(n, Ordering::SeqCst);
        d.store(total.unwrap_or(0), Ordering::SeqCst);
    })
    .await
    .unwrap();
    assert_eq!(received.load(Ordering::SeqCst), payload().len() as u64);
    assert_eq!(declared.load(Ordering::SeqCst), payload().len() as u64);
}

#[tokio::test]
async fn check_sends_exactly_one_get_to_the_pinned_path() {
    let server = serve("0.4.1", &payload_signature(), "", payload());
    offered(&server, "0.4.0").await;
    let log = server.log();
    assert_eq!(log.len(), 1, "exactly one request: {log:?}");
    let request = &log[0];
    assert_eq!(request.method, "GET");
    assert_eq!(request.path, "/latest.json");
    assert_eq!(request.query, None, "no query string");
    for (name, _) in &request.headers {
        assert!(
            !matches!(name.as_str(), "cookie" | "authorization"),
            "unexpected header {name}"
        );
    }
    let agent = request
        .headers
        .iter()
        .find(|(n, _)| n == "user-agent")
        .map(|(_, v)| v.as_str())
        .unwrap_or_default();
    assert!(agent.starts_with("tauri-plugin-updater/"), "{agent}");
    let accept = request
        .headers
        .iter()
        .find(|(n, _)| n == "accept")
        .map(|(_, v)| v.as_str());
    assert_eq!(accept, Some("application/json"));
}

#[tokio::test]
async fn redirect_hops_are_counted_and_followed() {
    // GitHub's `latest/download` answers with a redirect; each hop shows up in the log
    let server = FakeFeed::start_with(|addr| {
        let download = format!("http://{addr}/dl/payload.bin");
        vec![
            ("/latest.json", Route::Redirect("/cdn/feed.json".into())),
            (
                "/cdn/feed.json",
                Route::Json(manifest("0.4.1", &download, &payload_signature(), "")),
            ),
        ]
    });
    offered(&server, "0.4.0").await;
    let log = server.log();
    let paths: Vec<&str> = log.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["/latest.json", "/cdn/feed.json"]);
    assert!(log.iter().all(|r| r.query.is_none()));
}

/// A URL of `server` under another host name than the `127.0.0.1` the feed is configured with.
fn on_another_host(server: &FakeFeed, path: &str) -> String {
    format!("http://localhost:{}{path}", server.addr.port())
}

#[tokio::test]
async fn redirect_to_another_host_accepts_only_signed_bytes() {
    // GitHub answers the feed and the file with redirects to its asset host, so another host is
    // normal and only the signature decides
    let mut tampered = payload();
    tampered[100] ^= 0x01;
    for (served, expected) in [
        (payload(), Ok(payload())),
        (tampered, Err(FeedError::BadSignature)),
    ] {
        let files = FakeFeed::start(vec![("/cdn/payload.bin", Route::Body(served))]);
        let file = on_another_host(&files, "/cdn/payload.bin");
        let mid = FakeFeed::start_with(|addr| {
            let download = format!("http://localhost:{}/dl/payload.bin", addr.port());
            vec![
                (
                    "/cdn/feed.json",
                    Route::Json(manifest("9.9.9", &download, &payload_signature(), "")),
                ),
                ("/dl/payload.bin", Route::Redirect(file)),
            ]
        });
        let front = FakeFeed::start(vec![(
            "/latest.json",
            Route::Redirect(on_another_host(&mid, "/cdn/feed.json")),
        )]);
        let result = download_of(&front, &test_pubkey(), Limits::production()).await;
        assert_eq!(result, expected);
        let reached: Vec<String> = files.log().into_iter().map(|r| r.path).collect();
        assert_eq!(
            reached,
            ["/cdn/payload.bin"],
            "the other host served the file"
        );
    }
}

#[tokio::test]
async fn truncated_download_is_refused() {
    let short = payload()[..4096].to_vec();
    // the server says the short body is the whole file: only the signature can tell
    let (result, _server) =
        download_from_route(Route::Body(short.clone()), Limits::production()).await;
    assert_eq!(result.unwrap_err(), FeedError::BadSignature);
    // the connection ends before the announced length
    let (result, _server) = download_from_route(
        Route::Truncated {
            declared: 8192,
            body: short,
        },
        Limits::production(),
    )
    .await;
    assert_eq!(result.unwrap_err(), FeedError::DownloadRefused("failed"));
}

#[tokio::test]
async fn signature_without_a_version_is_refused() {
    // a genuine signature of this very file by the pinned (test) key, made without
    // `--app-version`, so nothing binds it to the announced version
    let signature = fs::read_to_string(
        manifest_dir().join("../crates/fernjo-core/tests/fixtures/update-sig/no-app-version.sig"),
    )
    .unwrap();
    let server = serve("9.9.9", signature.trim(), "", payload());
    let err = download_of(&server, &test_pubkey(), Limits::production())
        .await
        .unwrap_err();
    assert_eq!(err.code(), "signature_invalid");
}

#[tokio::test]
async fn notes_come_back_cleaned() {
    let server = serve(
        "0.4.1",
        &payload_signature(),
        "Fixed\r\n\u{1b}[31mred\u{1b}[0m\u{7}bell\nline",
        payload(),
    );
    let update = offered(&server, "0.4.0").await;
    assert!(!update.notes.contains('\u{1b}'));
    assert!(!update.notes.contains('\u{7}'));
    assert!(!update.notes.contains('\r'));
    assert!(update.notes.contains("Fixed\n"));
}
