//! Guards the security-relevant parts of the Tauri configuration.

use std::{fs, path::Path};

use serde_json::Value;

fn load(relative: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

#[test]
fn csp_is_strict() {
    let conf = load("tauri.conf.json");
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp must be a string");
    assert!(csp.contains("default-src 'self'"), "csp: {csp}");
    assert!(
        csp.contains("connect-src ipc: http://ipc.localhost"),
        "Tauri IPC needs connect-src; csp: {csp}"
    );
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    for token in csp.split(|c: char| c.is_whitespace() || c == ';') {
        assert_ne!(token, "*", "csp must not use a wildcard source: {csp}");
        if token.starts_with("http:") || token.starts_with("https:") {
            assert_eq!(
                token, "http://ipc.localhost",
                "the only allowed web origin is the Tauri IPC host; csp: {csp}"
            );
        }
    }
}

#[test]
fn webview2_arguments_keep_wry_defaults_and_turn_off_the_component_updater() {
    let conf = load("tauri.conf.json");
    let args = conf["app"]["windows"][0]["additionalBrowserArgs"]
        .as_str()
        .expect("app.windows[0].additionalBrowserArgs must be a string");
    // Setting the key replaces wry's own default, so the three features it turns off stay.
    assert!(
        args.contains("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection"),
        "wry's default disable-features must be kept: {args}"
    );
    // The WebView2 runtime's component updater is the one background connection
    // (Docs/DECISIONS.md, "M7: WebView2 background connection").
    assert!(args.contains("--disable-component-update"), "{args}");
    assert!(
        !args.contains("remote-debugging-port") && !args.contains("--no-sandbox"),
        "no debugging or sandbox flags in the shipped arguments: {args}"
    );
}

#[test]
fn global_tauri_is_off() {
    let conf = load("tauri.conf.json");
    let flag = &conf["app"]["withGlobalTauri"];
    assert!(
        flag.is_null() || *flag == Value::Bool(false),
        "withGlobalTauri: {flag}"
    );
}

/// Every capability file in `capabilities/`, as (file name, parsed JSON).
fn capability_files() -> Vec<(String, Value)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut files: Vec<(String, Value)> = fs::read_dir(&dir)
        .expect("read capabilities")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| {
            let name = p.file_name().expect("name").to_string_lossy().into_owned();
            let text = fs::read_to_string(&p).expect("read capability");
            (name, serde_json::from_str(&text).expect("capability json"))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

fn permissions_of(cap: &Value) -> Vec<String> {
    cap["permissions"]
        .as_array()
        .expect("permissions array")
        .iter()
        .map(|p| p.as_str().expect("string permission").to_owned())
        .collect()
}

#[test]
fn capability_has_no_core_default_and_no_updater_permission() {
    let files = capability_files();
    assert!(!files.is_empty());
    for (name, cap) in &files {
        for permission in permissions_of(cap) {
            assert_ne!(permission, "core:default", "{name} grants core:default");
            assert!(
                !permission.starts_with("updater:"),
                "{name} grants {permission}: the update flow runs in Rust commands"
            );
        }
    }
}

/// The webview needs two permissions: `listen`/`unlisten` of `@tauri-apps/api/event`
/// (`plugin:event|listen`, `plugin:event|unlisten`). App commands (`invoke`) need none while the
/// app has no ACL manifest, and a `Channel` fetch is exempt from the ACL. The UI uses no window,
/// webview, app, path, menu, tray, image or resources API (full screen is a CSS layout change).
/// Details: `Docs/DECISIONS.md` "M13: capability narrowing".
#[test]
fn capability_lists_exactly_the_pinned_permissions() {
    let files = capability_files();
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["default.json"], "no other capability file");
    let cap = &files[0].1;
    assert_eq!(
        permissions_of(cap),
        ["core:event:allow-listen", "core:event:allow-unlisten"]
    );
    assert_eq!(cap["windows"], serde_json::json!(["main"]));
    assert!(cap.get("webviews").is_none() && cap.get("remote").is_none());
}

#[test]
fn bundle_is_active_with_exactly_the_planned_targets() {
    let conf = load("tauri.conf.json");
    assert_eq!(conf["bundle"]["active"], Value::Bool(true));
    assert_eq!(
        conf["bundle"]["targets"],
        serde_json::json!(["nsis", "dmg", "appimage", "deb"])
    );
}

#[test]
fn bundle_icons_exist_on_disk() {
    let conf = load("tauri.conf.json");
    let icons = conf["bundle"]["icon"]
        .as_array()
        .expect("bundle.icon must be an array");
    assert!(!icons.is_empty(), "bundle.icon must not be empty");
    for icon in icons {
        let relative = icon.as_str().expect("bundle.icon entries are strings");
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        assert!(path.is_file(), "missing icon {}", path.display());
    }
}

/// Without `installerIcon` the bundler uses the NSIS default icon for `fernjo_<v>_x64-setup.exe`.
#[test]
fn nsis_installer_uses_the_app_icon() {
    let conf = load("tauri.conf.json");
    let icon = conf["bundle"]["windows"]["nsis"]["installerIcon"]
        .as_str()
        .expect("bundle.windows.nsis.installerIcon must be set to a path");
    assert_eq!(icon, "icons/icon.ico", "the installer carries the app icon");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(icon);
    assert!(path.is_file(), "missing installer icon {}", path.display());
    let bundle_icons = conf["bundle"]["icon"]
        .as_array()
        .expect("bundle.icon must be an array");
    assert!(
        bundle_icons.iter().any(|i| i.as_str() == Some(icon)),
        "the installer icon must be one of bundle.icon: {bundle_icons:?}"
    );
}

#[test]
fn version_comes_from_cargo_not_from_the_conf() {
    let conf = load("tauri.conf.json");
    assert!(
        conf.get("version").is_none(),
        "tauri.conf.json must not set a version; Cargo.toml is the single source"
    );
}

const ENDPOINT: &str = "https://github.com/benjo-code/fernjo/releases/latest/download/latest.json";
/// The owner's update public key (key id 15493CC4F35D089F, Docs/DECISIONS.md, "M13: update signing
/// and the updater plugin"). The private half is never in the repository.
const UPDATE_PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE1NDkzQ0M0RjM1RDA4OUYKUldTZkNGM3p4RHhKRmZYQjlQaGNSZVRCNWdKejJvZTg0TWh1bEl3VWorRmJKWCtEM0I1VTlKVSsK";

#[test]
fn updater_config_is_pinned() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let conf = load("tauri.conf.json");
    let updater = conf["plugins"]["updater"]
        .as_object()
        .expect("plugins.updater must be set");
    let mut keys: Vec<&str> = updater.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["endpoints", "pubkey", "requireSignedVersion"],
        "no other updater keys (no dangerous* switch, no allowDowngrades)"
    );
    assert_eq!(updater["endpoints"], serde_json::json!([ENDPOINT]));
    assert_eq!(updater["requireSignedVersion"], Value::Bool(true));
    let pubkey = updater["pubkey"].as_str().unwrap();
    assert_eq!(pubkey, UPDATE_PUBKEY);
    let text = String::from_utf8(STANDARD.decode(pubkey).expect("base64")).expect("utf-8");
    assert!(
        text.starts_with("untrusted comment: minisign public key: "),
        "a minisign public key text: {text}"
    );
    // the committed test key must never be the real one
    let test_key = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update-test-key/test-only.pub"),
    )
    .unwrap();
    assert_ne!(
        pubkey,
        test_key.trim(),
        "the test key is not the update key"
    );
    assert!(
        conf["bundle"]["createUpdaterArtifacts"].is_null(),
        "no updater artifacts: the build job needs no private key"
    );
    let text = serde_json::to_string(&conf).unwrap();
    for key in [
        "dangerousInsecureTransportProtocol",
        "signCommand",
        "signingIdentity",
        "certificateThumbprint",
        "TAURI_SIGNING_PRIVATE_KEY",
        "untrusted comment: rsign encrypted secret key",
    ] {
        assert!(!text.contains(key), "{key} must not appear in the conf");
    }
}

#[test]
fn update_test_hooks_are_not_a_default_feature() {
    let text =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    let mut in_features = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_features = line == "[features]";
        } else if in_features && line.starts_with("default") {
            assert!(!line.contains("update-test-hooks"), "{line}");
        }
    }
    assert!(
        text.contains("update-test-hooks = []"),
        "the feature exists and enables nothing else"
    );
}

#[test]
fn notices_are_a_bundle_resource() {
    let conf = load("tauri.conf.json");
    let resources = conf["bundle"]["resources"]
        .as_array()
        .expect("bundle.resources must be an array");
    let relative = "resources/THIRD-PARTY-NOTICES.md";
    assert!(
        resources.iter().any(|r| r.as_str() == Some(relative)),
        "bundle.resources must list {relative}: {resources:?}"
    );
    // tauri-build checks resource paths at compile time, so a committed placeholder must exist
    // (the release build overwrites it with the generated notices).
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    assert!(path.is_file(), "missing resource {}", path.display());
}

#[test]
fn lib_registers_the_navigation_guard_after_single_instance() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = fs::read_to_string(&path).unwrap();
    let single = text
        .find(".plugin(tauri_plugin_single_instance::init")
        .expect("lib.rs must register the single-instance plugin");
    let first = text.find(".plugin(").expect("lib.rs registers plugins");
    assert_eq!(first, single, "single-instance must stay the first plugin");
    let guard = text
        .find(".plugin(nav_guard::plugin())")
        .expect("lib.rs must register nav_guard::plugin()");
    assert!(
        guard > single,
        "the navigation guard comes after single-instance"
    );
    assert!(text.contains("mod nav_guard;"));
}

/// `form-action`, `base-uri` and `object-src` do not fall back to `default-src`; M11 review #64.
#[test]
fn csp_closes_the_directives_default_src_does_not_cover() {
    let conf = load("tauri.conf.json");
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    let directives: Vec<&str> = csp.split(';').map(str::trim).collect();
    for wanted in [
        "form-action 'none'",
        "base-uri 'self'",
        "object-src 'none'",
        "frame-ancestors 'none'",
    ] {
        assert!(directives.contains(&wanted), "csp lacks {wanted}: {csp}");
    }
}

/// WebView2's general autofill ("Suggestions") stays off so no typed or revealed value is
/// offered back from, or saved into, the profile; M11 review #63.
#[test]
fn main_window_turns_general_autofill_off() {
    let conf = load("tauri.conf.json");
    assert_eq!(
        conf["app"]["windows"][0]["generalAutofillEnabled"],
        Value::Bool(false),
        "app.windows[0].generalAutofillEnabled must be false"
    );
}
