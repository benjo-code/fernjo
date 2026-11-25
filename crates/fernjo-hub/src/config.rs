//! `config.json`: only the SHA-256 of the hub secret. The secret itself is printed once.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::write_atomic;

const CONFIG_FILE: &str = "config.json";
const MAX_CONFIG_LEN: u64 = 4096;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Config {
    secret_sha256: String,
}

fn path(dir: &Path) -> PathBuf {
    dir.join(CONFIG_FILE)
}

#[must_use]
pub fn hash_secret(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.as_bytes()).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn new_secret() -> io::Result<String> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn store_hash(dir: &Path, secret: &str) -> io::Result<()> {
    let cfg = Config {
        secret_sha256: hex(&hash_secret(secret)),
    };
    let json = serde_json::to_vec_pretty(&cfg).map_err(io::Error::other)?;
    write_atomic(&path(dir), &json)
}

/// Creates the folder and `config.json` with a new secret's hash; returns the secret (shown once).
/// Refuses to overwrite an existing config.
pub fn init(dir: &Path) -> io::Result<String> {
    fs::create_dir_all(dir)?;
    if path(dir).exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "already initialised; use rotate-secret",
        ));
    }
    let secret = new_secret()?;
    store_hash(dir, &secret)?;
    Ok(secret)
}

/// Replaces the stored hash with that of a new secret and returns it. A running `serve` keeps the
/// old hash until it is restarted.
pub fn rotate_secret(dir: &Path) -> io::Result<String> {
    load_secret_hash(dir)?;
    let secret = new_secret()?;
    store_hash(dir, &secret)?;
    Ok(secret)
}

pub fn load_secret_hash(dir: &Path) -> io::Result<[u8; 32]> {
    let file = fs::File::open(path(dir)).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            io::Error::new(io::ErrorKind::NotFound, "not initialised; run `init` first")
        } else {
            e
        }
    })?;
    let damaged = || io::Error::new(io::ErrorKind::InvalidData, "config.json is damaged");
    let mut raw = Vec::new();
    file.take(MAX_CONFIG_LEN + 1).read_to_end(&mut raw)?;
    if raw.len() as u64 > MAX_CONFIG_LEN {
        return Err(damaged());
    }
    let cfg: Config = serde_json::from_slice(&raw).map_err(|_| damaged())?;
    unhex(&cfg.secret_sha256).ok_or_else(damaged)
}
