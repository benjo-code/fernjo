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
    assert!(!csp.contains("unsafe-eval"), "csp: {csp}");
    assert!(!csp.contains("http:"), "csp: {csp}");
    assert!(!csp.contains("https:"), "csp: {csp}");
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
