//! Checks an update signature the way the app's updater does (release tooling, not app code).
//!
//! `cargo run -p fernjo-core --example verify_update_signature -- <tauri.conf.json> <file> <file.sig> [--app-version <v>]`
//!
//! Exit 0 only when `<file.sig>` verifies `<file>` against `plugins.updater.pubkey` of the conf. Both
//! are base64-wrapped minisign texts, as `tauri signer` writes them; the decoding and the call
//! mirror `verify_signature` of `tauri-plugin-updater` 2.14.1. With `--app-version <v>` the
//! signature's trusted comment must also carry a `version:` field equal to `<v>` (the check the
//! plugin makes for `requireSignedVersion`: a signature made without `tauri signer sign
//! --app-version` fails it). The release workflow runs this on every signed artifact.

use std::path::Path;
use std::process::ExitCode;

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};
use serde::Deserialize;

#[derive(Deserialize)]
struct Conf {
    plugins: Plugins,
}

#[derive(Deserialize)]
struct Plugins {
    updater: Updater,
}

#[derive(Deserialize)]
struct Updater {
    pubkey: String,
}

/// The public key text of a `tauri.conf.json` (still base64-wrapped).
fn pubkey_of(conf_json: &str) -> Result<String, String> {
    let conf: Conf = serde_json::from_str(conf_json)
        .map_err(|e| format!("the conf has no plugins.updater.pubkey: {e}"))?;
    Ok(conf.plugins.updater.pubkey)
}

/// Same as the plugin's `base64_to_string` (no trimming: the plugin decodes the configured public
/// key as it is, so a key with stray whitespace in the configuration fails here as it fails there).
fn base64_to_string(text: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| format!("not valid base64: {e}"))?;
    String::from_utf8(bytes).map_err(|_| "not valid UTF-8 after base64".to_string())
}

/// Same as the plugin's `signed_version`: the `version:` field of the tab separated trusted
/// comment.
fn signed_version(trusted_comment: &str) -> Option<&str> {
    trusted_comment
        .split('\t')
        .find_map(|field| field.strip_prefix("version:"))
}

/// Equal as semver-looking versions, with an optional leading `v` on either side (the plugin
/// compares parsed semver and falls back to a literal comparison; release versions are plain
/// `MAJOR.MINOR.PATCH`, so a trimmed comparison is the same thing).
fn same_version(signed: &str, wanted: &str) -> bool {
    signed.trim_start_matches('v') == wanted.trim_start_matches('v')
}

/// Verifies `data` against the signature and, when `app_version` is given, the signed version.
fn verify(
    conf_json: &str,
    data: &[u8],
    signature_text: &str,
    app_version: Option<&str>,
) -> Result<(), String> {
    let pubkey_text =
        base64_to_string(&pubkey_of(conf_json)?).map_err(|e| format!("pubkey: {e}"))?;
    let public_key = PublicKey::decode(&pubkey_text).map_err(|e| format!("pubkey: {e}"))?;
    // A `.sig` file may end with a newline and `latest.json` carries its trimmed text, so the
    // signature is trimmed here; the public key is not.
    let signature_decoded =
        base64_to_string(signature_text.trim()).map_err(|e| format!("signature: {e}"))?;
    let signature = Signature::decode(&signature_decoded).map_err(|e| format!("signature: {e}"))?;
    public_key
        // no legacy mode: the app refuses it too (M14 review #85)
        .verify(data, &signature, false)
        .map_err(|e| format!("signature does not verify: {e}"))?;
    // Only after `verify` is the trusted comment usable (its global signature was checked there).
    if let Some(wanted) = app_version {
        match signed_version(signature.trusted_comment()) {
            None => return Err("the signature carries no signed app version".to_string()),
            Some(signed) if same_version(signed, wanted) => {}
            Some(signed) => {
                return Err(format!(
                    "the signature is for version {signed}, expected {wanted}"
                ));
            }
        }
    }
    Ok(())
}

fn read(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(Path::new(path)).map_err(|e| format!("cannot read {path}: {e}"))
}

fn run(args: &[String]) -> Result<(), String> {
    let usage =
        "usage: verify_update_signature <tauri.conf.json> <file> <file.sig> [--app-version <v>]";
    let (conf, file, sig, app_version) = match args {
        [conf, file, sig] => (conf, file, sig, None),
        [conf, file, sig, flag, version] if flag == "--app-version" => {
            (conf, file, sig, Some(version.as_str()))
        }
        _ => return Err(usage.to_string()),
    };
    let conf_text = String::from_utf8(read(conf)?).map_err(|_| format!("{conf} is not UTF-8"))?;
    let sig_text = String::from_utf8(read(sig)?).map_err(|_| format!("{sig} is not UTF-8"))?;
    verify(&conf_text, &read(file)?, &sig_text, app_version)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => {
            println!("signature ok");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/update-sig")
            .join(name);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn payload() -> Vec<u8> {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update-sig/payload.bin");
        std::fs::read(path).unwrap()
    }

    #[test]
    fn verifies_a_signature_made_with_the_test_key() {
        let result = verify(
            &fixture("tauri.conf.json"),
            &payload(),
            &fixture("with-app-version.sig"),
            None,
        );
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn rejects_a_signature_from_another_key() {
        let result = verify(
            &fixture("tauri.conf.json"),
            &payload(),
            &fixture("other-key.sig"),
            None,
        );
        assert!(result.unwrap_err().contains("does not verify"));
        // The same signature is fine against the key that made it.
        assert_eq!(
            verify(
                &fixture("other-conf.json"),
                &payload(),
                &fixture("other-key.sig"),
                None
            ),
            Ok(())
        );
    }

    /// BLAKE2b-512 of `payload.bin` (`b2sum`): what a prehashed signature of it really signs.
    const PAYLOAD_BLAKE2B: &str = "1dd1daa565d43df90bed59272794b990cce97df593cc90a04955a14684030a64b4398d9b050c3b45b12c116ec573081f5e382ce8366bd628d9e3befa923185b0";

    fn payload_hash() -> Vec<u8> {
        (0..PAYLOAD_BLAKE2B.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&PAYLOAD_BLAKE2B[i..i + 2], 16).unwrap())
            .collect()
    }

    /// The signature with its algorithm bytes switched from `ED` (prehashed) to `Ed` (legacy), which
    /// neither signature inside it covers.
    fn switched_to_legacy(signature: &str) -> String {
        let b64 = &base64::engine::general_purpose::STANDARD;
        let text = String::from_utf8(b64.decode(signature.trim()).unwrap()).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let mut raw = b64.decode(&lines[1]).unwrap();
        assert_eq!(&raw[..2], b"ED");
        raw[1] = b'd';
        lines[1] = b64.encode(raw);
        b64.encode(lines.join("\n") + "\n")
    }

    #[test]
    fn rejects_a_signature_switched_to_legacy_mode() {
        let conf = fixture("tauri.conf.json");
        let signature = switched_to_legacy(&fixture("with-app-version.sig"));
        // in legacy mode the 64-byte hash of the signed file would pass as if it were the file
        let result = verify(&conf, &payload_hash(), &signature, Some("9.9.9"));
        assert!(result.unwrap_err().contains("does not verify"));
    }

    #[test]
    fn rejects_a_modified_file() {
        let mut data = payload();
        data[0] ^= 1;
        let result = verify(
            &fixture("tauri.conf.json"),
            &data,
            &fixture("with-app-version.sig"),
            None,
        );
        assert!(result.unwrap_err().contains("does not verify"));
    }

    #[test]
    fn app_version_must_match_the_signed_one() {
        let conf = fixture("tauri.conf.json");
        let sig = fixture("with-app-version.sig");
        assert_eq!(verify(&conf, &payload(), &sig, Some("9.9.9")), Ok(()));
        assert_eq!(verify(&conf, &payload(), &sig, Some("v9.9.9")), Ok(()));
        let err = verify(&conf, &payload(), &sig, Some("9.9.8")).unwrap_err();
        assert!(err.contains("9.9.9") && err.contains("9.9.8"));
    }

    #[test]
    fn signature_without_app_version_fails_the_version_check() {
        let conf = fixture("tauri.conf.json");
        let sig = fixture("no-app-version.sig");
        // It verifies as a signature ...
        assert_eq!(verify(&conf, &payload(), &sig, None), Ok(()));
        // ... but carries no version to bind.
        let err = verify(&conf, &payload(), &sig, Some("9.9.9")).unwrap_err();
        assert!(err.contains("no signed app version"));
    }

    #[test]
    fn reads_the_version_field_like_the_plugin() {
        assert_eq!(
            signed_version("timestamp:1\tfile:a.zip\tversion:1.2.3"),
            Some("1.2.3")
        );
        assert_eq!(signed_version("timestamp:1\tfile:app-version:2.zip"), None);
    }

    #[test]
    fn a_pubkey_with_stray_whitespace_is_refused_like_the_plugin_does() {
        let conf = fixture("tauri.conf.json");
        let sig = fixture("with-app-version.sig");
        let padded = conf.replace("\"pubkey\": \"", "\"pubkey\": \" ");
        assert_ne!(padded, conf);
        let err = verify(&padded, &payload(), &sig, None).unwrap_err();
        assert!(err.starts_with("pubkey: not valid base64"), "{err}");
        // a signature file ends with a newline; the signature is trimmed, the public key is not
        let with_newline = format!("{}\n", sig.trim());
        assert_eq!(verify(&conf, &payload(), &with_newline, None), Ok(()));
    }

    #[test]
    fn a_conf_without_a_pubkey_is_an_error() {
        let err = verify("{}", &payload(), &fixture("with-app-version.sig"), None).unwrap_err();
        assert!(err.contains("pubkey"));
    }

    #[test]
    fn arguments_are_checked() {
        assert!(run(&[]).unwrap_err().starts_with("usage"));
        let args: Vec<String> = ["a", "b", "c", "--other", "1"].map(String::from).to_vec();
        assert!(run(&args).unwrap_err().starts_with("usage"));
    }
}
