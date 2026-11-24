//! Server certificates this device has accepted, pinned per `host:port`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::RdpError;

/// Serialises every read-modify-write of a certificate file in this process (one lock for all
/// files: the writes are rare and short). Another process is not covered; its changes are picked
/// up by the next read.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// Longest host accepted: the DNS limit for a name in text form.
const MAX_HOST_LEN: usize = 255;

/// Largest certificate file the store reads: 1 MiB, far above any real list of pins.
pub(crate) const MAX_FILE_LEN: u64 = 1024 * 1024;

/// What the stored pin says about a presented certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertCheck {
    /// Nothing is pinned for the host and port.
    Unknown,
    /// The presented fingerprint is the pinned one.
    Match,
    /// Another fingerprint is pinned (the old one).
    Changed { old: String },
}

/// The pins of one device. [`CertStore::in_memory`] has no file; its clones share the pins.
#[derive(Debug, Clone)]
pub struct CertStore {
    path: Option<PathBuf>,
    memory: Arc<Mutex<Vec<u8>>>,
}

impl CertStore {
    /// The pins in the file at `path`; a missing file counts as empty and is created on the first
    /// pin.
    pub fn open(path: PathBuf) -> CertStore {
        CertStore {
            path: Some(path),
            memory: Arc::default(),
        }
    }

    pub fn in_memory() -> CertStore {
        CertStore {
            path: None,
            memory: Arc::default(),
        }
    }

    pub fn check(&self, host: &str, port: u16, fingerprint: &str) -> Result<CertCheck, RdpError> {
        let key = pin_key(host, port)?;
        check_fingerprint(fingerprint)?;
        let bytes = self.read()?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(match stored_fingerprint(&text, &key) {
            None => CertCheck::Unknown,
            Some(old) if old == fingerprint => CertCheck::Match,
            Some(old) => CertCheck::Changed {
                old: old.to_string(),
            },
        })
    }

    /// Pins `fingerprint` for the host and port, replacing an older pin.
    pub fn pin(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), RdpError> {
        let key = pin_key(host, port)?;
        check_fingerprint(fingerprint)?;
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = self.read()?;
        let text = String::from_utf8_lossy(&bytes);
        let mut out = String::with_capacity(text.len() + key.len() + fingerprint.len() + 2);
        for line in text.lines().filter(|l| line_key(l) != Some(key.as_str())) {
            out.push_str(line);
            out.push('\n');
        }
        out.push_str(&format!("{key} {fingerprint}\n"));
        self.write(out.as_bytes())
    }

    /// Same as [`CertStore::pin`]; the name says why it is called: the user accepted a changed
    /// certificate.
    pub fn replace(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), RdpError> {
        self.pin(host, port, fingerprint)
    }

    fn read(&self) -> Result<Vec<u8>, RdpError> {
        let Some(path) = &self.path else {
            return Ok(self.memory.lock().map_err(|_| RdpError::Io)?.clone());
        };
        match read_capped(path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(RdpError::Io),
        }
    }

    fn write(&self, contents: &[u8]) -> Result<(), RdpError> {
        match &self.path {
            Some(path) => write_atomic(path, contents),
            None => {
                *self.memory.lock().map_err(|_| RdpError::Io)? = contents.to_vec();
                Ok(())
            }
        }
    }
}

/// Whether `host` can be used as a host name here: not empty, at most 255 bytes, no whitespace or
/// control characters.
pub(crate) fn valid_host(host: &str) -> bool {
    !(host.is_empty()
        || host.len() > MAX_HOST_LEN
        || host.chars().any(|c| c.is_whitespace() || c.is_control()))
}

/// `host:port`, the host lowercased. A host that is empty, longer than 255 bytes or holds
/// whitespace or a control character is `invalid_host` (it could not be one field of a line).
fn pin_key(host: &str, port: u16) -> Result<String, RdpError> {
    if !valid_host(host) {
        return Err(RdpError::InvalidHost);
    }
    Ok(format!("{}:{port}", host.to_ascii_lowercase()))
}

/// A fingerprint must be one field of a line.
fn check_fingerprint(fingerprint: &str) -> Result<(), RdpError> {
    if fingerprint.is_empty()
        || fingerprint.len() > 256
        || fingerprint
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(RdpError::Protocol);
    }
    Ok(())
}

/// The first field of a pin line (`host:port`), or `None` for a comment or an unusable line.
fn line_key(line: &str) -> Option<&str> {
    if line.starts_with('#') {
        return None;
    }
    let mut fields = line.split_whitespace();
    let (key, _fingerprint) = (fields.next()?, fields.next()?);
    Some(key)
}

fn stored_fingerprint<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines()
        .filter(|l| line_key(l) == Some(key))
        .filter_map(|l| l.split_whitespace().nth(1))
        .next_back()
}

/// Reads `path`, refusing a file above [`MAX_FILE_LEN`] (checked before and while reading).
fn read_capped(path: &Path) -> io::Result<Vec<u8>> {
    fernjo_fs::read_capped(path, MAX_FILE_LEN)
}

/// Writes through `fernjo_fs::write_atomic` with the data-file options: missing folders created
/// private, a random-named temp file, owner-only modes on Unix, the previous file kept as `.bak`
/// (never written through a symlink at that name), rename, folder sync on Unix.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), RdpError> {
    fernjo_fs::write_atomic(path, contents, fernjo_fs::WriteOptions::data_file())
        .map_err(|_| RdpError::Io)
}

#[cfg(test)]
mod tests;
