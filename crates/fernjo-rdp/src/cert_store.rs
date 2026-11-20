//! Server certificates this device has accepted, pinned per `host:port`.

use std::ffi::OsString;
use std::fs;
use std::io::{self, Read, Write};
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
    let too_large = || io::Error::new(io::ErrorKind::InvalidData, "the file is too large");
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_LEN {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_LEN + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_LEN {
        return Err(too_large());
    }
    Ok(bytes)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Creates a new file (`create_new`: never an existing file, never through a symlink at that
/// name) readable and writable by its owner only on Unix (mode 0600); elsewhere the folder's
/// inherited ACL applies.
fn create_new_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// A temp file next to `path` under a random name, so nobody can plant the name beforehand; a
/// name that is taken anyway is never reused. (Same approach as the SSH known hosts writer; this
/// crate does not depend on `fernjo-vault` or `fernjo-ssh`, so the few lines are repeated.)
fn create_temp_file(path: &Path) -> io::Result<(PathBuf, fs::File)> {
    let mut last = None;
    for _ in 0..8 {
        let mut random = [0u8; 8];
        getrandom::fill(&mut random).map_err(|e| io::Error::other(e.to_string()))?;
        let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = with_suffix(path, &format!(".{hex}.tmp"));
        match create_new_private_file(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free temp file name")))
}

/// Unix: flushes the folder entry of the rename to disk. Elsewhere nothing to do.
fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// Creates `dir` and its missing parents; on Unix with mode 0700 (folders that already exist
/// keep theirs).
fn create_private_dir_all(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// Unix: mode 0600. Other platforms: no change.
fn owner_only(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Random-named temp file next to `path`, `sync_all`, current file copied to `.bak`, rename over
/// `path`, folder sync on Unix.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), RdpError> {
    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let mut tmp: Option<PathBuf> = None;
    let result = (|| -> io::Result<()> {
        if let Some(dir) = dir {
            create_private_dir_all(dir)?;
        }
        let (name, mut file) = create_temp_file(path)?;
        let name = tmp.insert(name);
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            let bak = with_suffix(path, ".bak");
            fs::copy(path, &bak)?;
            owner_only(&bak)?;
        }
        fs::rename(&*name, path)
    })();
    if result.is_err() {
        if let Some(tmp) = &tmp {
            let _ = fs::remove_file(tmp);
        }
        return Err(RdpError::Io);
    }
    // the file is in place; a failed folder sync must not report the write as failed
    if let Some(dir) = dir {
        let _ = sync_dir(dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
