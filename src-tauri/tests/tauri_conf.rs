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
fn global_tauri_is_off() {
    let conf = load("tauri.conf.json");
    let flag = &conf["app"]["withGlobalTauri"];
    assert!(
        flag.is_null() || *flag == Value::Bool(false),
        "withGlobalTauri: {flag}"
    );
}

#[test]
fn default_capability_is_core_only() {
    let cap = load("capabilities/default.json");
    assert_eq!(cap["permissions"], serde_json::json!(["core:default"]));
    assert_eq!(cap["windows"], serde_json::json!(["main"]));
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

#[test]
fn no_updater_and_no_signing_material() {
    let conf = load("tauri.conf.json");
    assert!(conf["plugins"]["updater"].is_null(), "no updater plugin");
    assert!(
        conf["bundle"]["createUpdaterArtifacts"].is_null(),
        "no updater artifacts"
    );
    let text = serde_json::to_string(&conf).unwrap();
    for key in ["signCommand", "signingIdentity", "certificateThumbprint"] {
        assert!(!text.contains(key), "{key} must not appear in the conf");
    }
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
