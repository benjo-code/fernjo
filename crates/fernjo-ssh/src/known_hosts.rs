//! OpenSSH-compatible `known_hosts` file. The file is shared with OpenSSH, so a rewrite only ever
//! touches the lines it owns; every other byte (comments, blank or garbage lines, CRLF, BOM) stays.
//!
//! Matching is done locally instead of through `russh::keys::known_hosts::known_host_keys_path`:
//! that helper fails the whole lookup on one unparsable key line, does not count comment lines in
//! the line numbers it reports, and mishandles CRLF and a leading BOM. Supported host patterns are
//! exact names (`host`, `[host]:port`) and hashed `|1|` entries; wildcards, negations and `@marker`
//! lines are left alone and never match.

use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::ssh_key::HashAlg;
use russh::keys::{Algorithm, PublicKey, parse_public_key_base64};
use sha1::Sha1;

use crate::SshError;

/// Serialises every read-modify-write of a `known_hosts` file in this process (one lock for all
/// files: the writes are rare and short). It stops two connects from losing each other's update.
/// Another process (OpenSSH, a sync tool) is not covered; its changes are picked up by the next read.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

const BOM: &[u8] = b"\xEF\xBB\xBF";

/// Largest `known_hosts` (or `known_hosts.confirmed`) that is read: 16 MiB. A line is about 100
/// to 750 bytes (an RSA 4096 key is the longest), so this holds tens of thousands of hosts, more
/// than any hand-kept or managed file; the whole file is read and scanned on every connect. A
/// larger file is refused (`Io`), like an unreadable one.
pub(crate) const MAX_KNOWN_HOSTS_LEN: u64 = 16 * 1024 * 1024;

/// Result of comparing a presented host key with the stored ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// No key of this algorithm is stored for the host and port.
    Unknown,
    /// The presented key is stored.
    Known,
    /// Keys of the same algorithm are stored, none equals the presented one. Their fingerprints.
    Changed { stored: Vec<String> },
}

/// `SHA256:<base64 without padding>`, the format of `ssh-keygen -lf`.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Handle on one `known_hosts` file. A missing file counts as empty. [`KnownHosts::in_memory`]
/// is a file-less variant (shared between its clones) for callers that have nowhere to write.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
    memory: Option<Arc<Mutex<Vec<u8>>>>,
}

/// One line that matches a host: where it lives in the file and what it stores.
struct Match {
    /// The whole line including its terminator.
    line: Range<usize>,
    /// The host-pattern field.
    hosts: Range<usize>,
    /// Indices of every entry within the comma-separated host field that matches the name (a hand
    /// edited `h,H` line can name the same host twice).
    entries: Vec<usize>,
    key: PublicKey,
}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            memory: None,
        }
    }

    /// An empty store that lives only as long as its handles; nothing touches the disk. Same
    /// matching, `add` and `replace` rules as the file-backed one.
    pub fn in_memory() -> Self {
        Self {
            path: PathBuf::new(),
            memory: Some(Arc::default()),
        }
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus, SshError> {
        let bytes = self.read()?;
        let same: Vec<PublicKey> = find_matches(&bytes, &host_port(host, port)?)
            .into_iter()
            .map(|m| m.key)
            .filter(|k| k.algorithm() == key.algorithm())
            .collect();
        if same.is_empty() {
            Ok(HostKeyStatus::Unknown)
        } else if same.iter().any(|k| k == key) {
            Ok(HostKeyStatus::Known)
        } else {
            Ok(HostKeyStatus::Changed {
                stored: same.iter().map(fingerprint).collect(),
            })
        }
    }

    /// Algorithms of the keys stored for the host and port, in file order, without duplicates.
    pub fn stored_algorithms(&self, host: &str, port: u16) -> Result<Vec<Algorithm>, SshError> {
        let bytes = self.read()?;
        let mut out: Vec<Algorithm> = Vec::new();
        for m in find_matches(&bytes, &host_port(host, port)?) {
            let alg = m.key.algorithm();
            if !out.contains(&alg) {
                out.push(alg);
            }
        }
        Ok(out)
    }

    /// The keys stored for the host and port, of every algorithm, in file order.
    pub fn stored_keys(&self, host: &str, port: u16) -> Result<Vec<PublicKey>, SshError> {
        let bytes = self.read()?;
        Ok(find_matches(&bytes, &host_port(host, port)?)
            .into_iter()
            .map(|m| m.key)
            .collect())
    }

    /// Appends the key unless it is already stored for the host and port.
    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        let name = host_port(host, port)?;
        self.modify(|bytes| {
            if find_matches(bytes, &name).iter().any(|m| &m.key == key) {
                return Ok(None);
            }
            let mut out = bytes.to_vec();
            append_line(&mut out, &name, key)?;
            Ok(Some(out))
        })
    }

    /// Records the key as trusted: whatever is stored for the host and port under the key's
    /// algorithm is replaced, a key of another algorithm stays. Same as [`KnownHosts::replace`],
    /// which also works when nothing is stored yet.
    pub fn trust(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        self.replace(host, port, key)
    }

    /// Drops the stored keys of the same algorithm for the host and port, then appends the new one.
    pub fn replace(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        let name = host_port(host, port)?;
        self.modify(|bytes| {
            let mut out = rewrite_without(bytes, &name, key);
            append_line(&mut out, &name, key)?;
            Ok(Some(out))
        })
    }

    /// Reads the file, lets `f` produce the new contents (`None`: nothing to write) and writes them,
    /// all under the process-wide lock.
    fn modify(
        &self,
        f: impl FnOnce(&[u8]) -> Result<Option<Vec<u8>>, SshError>,
    ) -> Result<(), SshError> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bytes = self.read()?;
        match f(&bytes)? {
            Some(out) => self.write(&out),
            None => Ok(()),
        }
    }

    fn write(&self, contents: &[u8]) -> Result<(), SshError> {
        match &self.memory {
            Some(memory) => {
                *memory.lock().map_err(|_| SshError::Io)? = contents.to_vec();
                Ok(())
            }
            None => {
                // leftovers of an interrupted earlier save of this file (best effort); this runs
                // under `WRITE_LOCK` on the blocking pool, like the write itself
                fernjo_fs::sweep_stale_temps(&self.path, fernjo_fs::STALE_TEMP_AGE);
                write_atomic(&self.path, contents)
            }
        }
    }

    fn read(&self) -> Result<Vec<u8>, SshError> {
        if let Some(memory) = &self.memory {
            return Ok(memory.lock().map_err(|_| SshError::Io)?.clone());
        }
        // capped, and never through a symlink or from a FIFO or device at the name (a planted
        // link would otherwise be read here and written back into the synced folder by `modify`)
        match fernjo_fs::read_capped(&self.path, MAX_KNOWN_HOSTS_LEN) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(SshError::Io),
        }
    }
}

/// Runs a `known_hosts` file operation on tokio's blocking pool, so a slow or hung file system
/// (a stalled network share) holds a blocking thread instead of one of the shared runtime's async
/// workers. A panicked or cancelled operation is `Io`.
pub(crate) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, SshError> + Send + 'static,
) -> Result<T, SshError> {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or(Err(SshError::Io))
}

/// `bytes` without the entries for `name` whose key has the algorithm of `key`. A line shared with
/// other hosts loses only this host's entries (every one of them, also a repeated name, so no stale
/// key stays for the host); a line left with no host is dropped entirely.
fn rewrite_without(bytes: &[u8], name: &str, key: &PublicKey) -> Vec<u8> {
    let drop_these: Vec<Match> = find_matches(bytes, name)
        .into_iter()
        .filter(|m| m.key.algorithm() == key.algorithm())
        .collect();
    let mut out = Vec::with_capacity(bytes.len() + 128);
    let mut pos = 0;
    // Matches come in file order and do not overlap.
    for m in &drop_these {
        out.extend_from_slice(&bytes[pos..m.line.start]);
        let kept: Vec<&[u8]> = bytes[m.hosts.clone()]
            .split(|&b| b == b',')
            .enumerate()
            .filter(|(i, _)| !m.entries.contains(i))
            .map(|(_, e)| e)
            .collect();
        if !kept.is_empty() {
            out.extend_from_slice(&bytes[m.line.start..m.hosts.start]);
            out.extend_from_slice(&kept.join(&b","[..]));
            out.extend_from_slice(&bytes[m.hosts.end..m.line.end]);
        }
        pos = m.line.end;
    }
    out.extend_from_slice(&bytes[pos..]);
    out
}

/// Longest host name accepted: the DNS limit for a name in text form.
const MAX_HOST_LEN: usize = 255;

/// Checks that `host` can be written to `known_hosts` as one plain host pattern. The accepted
/// forms are what a connection can actually resolve: DNS names (letters, digits, `-`, `_`, `.`,
/// a trailing dot; non-ASCII letters and digits are kept because the resolver gets the name
/// as typed, an internationalised name can also be entered as punycode) and IPv4/IPv6
/// literals (an IPv6 literal without brackets, optionally with a `%zone`). Everything else is
/// refused: empty or longer than 255 bytes, whitespace and control characters (a newline would
/// start a second line), `,` (would add an alias host), and `|`, `@`, `!`, `#`, `*`, `?`,
/// quotes, brackets and `/` (patterns, markers, comments, or the `[host]:port` form that
/// `host_port` generates itself).
///
/// # Errors
/// [`SshError::InvalidHost`].
pub fn validate_host(host: &str) -> Result<(), SshError> {
    if host.is_empty() || host.len() > MAX_HOST_LEN {
        return Err(SshError::InvalidHost);
    }
    let plain = |c: char| c.is_alphanumeric() || matches!(c, '.' | '-' | '_');
    let ok = if host.contains(':') {
        // IPv6 literal, optionally with a zone id ("fe80::1%eth0")
        let (addr, zone) = match host.split_once('%') {
            Some((addr, zone)) => (addr, Some(zone)),
            None => (host, None),
        };
        addr.parse::<std::net::Ipv6Addr>().is_ok()
            && zone.is_none_or(|z| !z.is_empty() && z.chars().all(plain))
    } else {
        host.chars().all(plain)
    };
    if ok {
        Ok(())
    } else {
        Err(SshError::InvalidHost)
    }
}

/// The host name as OpenSSH writes it: bare on port 22, `[host]:port` otherwise. Lowercased.
/// Refuses a host that [`validate_host`] does not accept.
fn host_port(host: &str, port: u16) -> Result<String, SshError> {
    validate_host(host)?;
    let host = host.to_ascii_lowercase();
    Ok(if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    })
}

/// Appends `name key` as a new line, using the file's line ending (CRLF if it has any).
fn append_line(bytes: &mut Vec<u8>, name: &str, key: &PublicKey) -> Result<(), SshError> {
    let eol: &[u8] = if bytes.windows(2).any(|w| w == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.extend_from_slice(eol);
    }
    let openssh = key.to_openssh().map_err(|_| SshError::Protocol)?;
    bytes.extend_from_slice(name.as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(openssh.as_bytes());
    bytes.extend_from_slice(eol);
    Ok(())
}

fn find_matches(bytes: &[u8], name: &str) -> Vec<Match> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |i| start + i + 1);
        if let Some(m) = parse_line(bytes, start..end, name) {
            out.push(m);
        }
        start = end;
    }
    out
}

/// Returns the match when the line is a usable key line (host, algorithm, parsable key) and one
/// of its host entries is `name`. Anything else is not ours and yields `None`.
fn parse_line(bytes: &[u8], line: Range<usize>, name: &str) -> Option<Match> {
    let mut from = line.start;
    if from == 0 && bytes.starts_with(BOM) {
        from = BOM.len();
    }
    let mut content_end = line.end;
    while content_end > from && matches!(bytes[content_end - 1], b'\n' | b'\r') {
        content_end -= 1;
    }
    let content = &bytes[from..content_end];
    if content.first() == Some(&b'#') {
        return None;
    }
    let mut fields: Vec<Range<usize>> = Vec::with_capacity(3);
    let mut i = 0;
    while i < content.len() && fields.len() < 3 {
        while i < content.len() && matches!(content[i], b' ' | b'\t') {
            i += 1;
        }
        let s = i;
        while i < content.len() && !matches!(content[i], b' ' | b'\t') {
            i += 1;
        }
        if i > s {
            fields.push(s..i);
        }
    }
    if fields.len() < 3 || content[fields[0].clone()].starts_with(b"@") {
        return None;
    }
    let hosts = (from + fields[0].start)..(from + fields[0].end);
    let entries: Vec<usize> = bytes[hosts.clone()]
        .split(|&b| b == b',')
        .enumerate()
        .filter(|(_, e)| entry_matches(e, name))
        .map(|(i, _)| i)
        .collect();
    if entries.is_empty() {
        return None;
    }
    let key_b64 = std::str::from_utf8(&content[fields[2].clone()]).ok()?;
    let key = parse_public_key_base64(key_b64).ok()?;
    Some(Match {
        line,
        hosts,
        entries,
        key,
    })
}

fn entry_matches(entry: &[u8], name: &str) -> bool {
    if let Some(rest) = entry.strip_prefix(b"|1|") {
        let mut parts = rest.split(|&b| b == b'|');
        let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else {
            return false;
        };
        let (Ok(salt), Ok(hash)) = (BASE64.decode(salt), BASE64.decode(hash)) else {
            return false;
        };
        let Ok(mut mac) = Hmac::<Sha1>::new_from_slice(&salt) else {
            return false;
        };
        mac.update(name.as_bytes());
        mac.verify_slice(&hash).is_ok()
    } else {
        entry.eq_ignore_ascii_case(name.as_bytes())
    }
}

/// Writes through `fernjo_fs::write_atomic` with the data-file options: missing folders created
/// private, a random-named temp file, owner-only modes on Unix, the previous file kept as `.bak`
/// (never written through a symlink at that name), rename, folder sync on Unix.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SshError> {
    fernjo_fs::write_atomic(path, contents, fernjo_fs::WriteOptions::data_file())
        .map_err(|_| SshError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    // Public keys generated with `ssh-keygen`; `FP_*` from `ssh-keygen -lf` on the same key.
    const ED_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIJ4tyazBiaSQKaes5aSfu9yuLDHNJEPyJiAHqg83VN5T";
    const FP_ED_A: &str = "SHA256:fJx8pIbcHzkR2+fd1fj6DszwZ7Uzsw82br7NCdiBVQg";
    const ED_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIMBS1jUJrjneokA1w91/JopLVVupB9FJxwYnQw8HiGVt";
    const FP_ED_B: &str = "SHA256:DLvtKR5kaQF5WDjJpqU3/ig+IxKNnmcCOKfAVVv2byA";
    const RSA: &str = "AAAAB3NzaC1yc2EAAAADAQABAAABAQDNBatQIjvOqKwNSQns0swa2yvfCxk6oY2NOLPV1MSMITEkEOr6AoxfsPEliwsJ8xSLMs7qZVOfpb/80vx2PULC+XqOkO2VZwal+aKS1r9YWhpD0w5TpPUz23wB3YPcnR2hzW20vy8Lg9xClIoPwejBtZvBC83nuNBQd+6BakS09oHdgS09v8VOxnIu5JfQ+qZGFK0tlDtF6/FjuKR1eVtWyG9TagXIZP3GbSypkk/moGeG56qgrmq/HgcASpQWXN7tRQ7cC6yHLK5u7z9fitNusfWjE1F3qF7036hhEy9GTXT+4kqjGDbLEhGaa+YDpSiItg7hxLLaID+IeZQukoPP";
    // `ssh-keygen -H` output for `hashed.example.com` with key ED_A.
    const HASHED_LINE: &str = "|1|Txm987nShyyi4b6xyXfmU+C8ZJg=|VSowLYepwoMoZTo/6dUHQ7h3+k8= ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ4tyazBiaSQKaes5aSfu9yuLDHNJEPyJiAHqg83VN5T";

    fn key(b64: &str) -> PublicKey {
        russh::keys::parse_public_key_base64(b64).unwrap()
    }

    fn setup() -> (tempfile::TempDir, KnownHosts, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts");
        (dir, KnownHosts::new(&path), path)
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[track_caller]
    fn expect_io<T: std::fmt::Debug>(result: Result<T, SshError>) {
        assert!(matches!(result, Err(SshError::Io)), "{result:?}");
    }

    #[test]
    fn fingerprint_matches_ssh_keygen() {
        assert_eq!(fingerprint(&key(ED_A)), FP_ED_A);
        assert_eq!(fingerprint(&key(ED_B)), FP_ED_B);
        assert!(fingerprint(&key(RSA)).starts_with("SHA256:o4tcvFNJ"));
        assert_eq!(
            fingerprint(&key(RSA)),
            "SHA256:o4tcvFNJetvCkrMMZE3EzNuJtx579lA1kjYgeRB7LRE"
        );
    }

    #[test]
    fn unknown_then_added_is_known() {
        let (_d, kh, _p) = setup();
        let k = key(ED_A);
        assert_eq!(
            kh.check("host.example", 22, &k).unwrap(),
            HostKeyStatus::Unknown
        );
        kh.add("host.example", 22, &k).unwrap();
        assert_eq!(
            kh.check("host.example", 22, &k).unwrap(),
            HostKeyStatus::Known
        );
        kh.add("host.example", 22, &k).unwrap();
        assert_eq!(
            kh.stored_algorithms("host.example", 22).unwrap().len(),
            1,
            "adding the same key twice must not duplicate it"
        );
    }

    #[test]
    fn different_key_of_same_type_is_changed_with_old_fingerprints() {
        let (_d, kh, _p) = setup();
        kh.add("h", 22, &key(ED_A)).unwrap();
        assert_eq!(
            kh.check("h", 22, &key(ED_B)).unwrap(),
            HostKeyStatus::Changed {
                stored: vec![FP_ED_A.to_string()]
            }
        );
    }

    #[test]
    fn other_key_type_is_unknown_not_changed() {
        let (_d, kh, _p) = setup();
        kh.add("h", 22, &key(ED_A)).unwrap();
        assert_eq!(
            kh.check("h", 22, &key(RSA)).unwrap(),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn port_other_than_22_uses_bracket_form_and_does_not_match_port_22() {
        let (_d, kh, p) = setup();
        kh.add("h", 2222, &key(ED_A)).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("[h]:2222 ssh-ed25519 "), "{text}");
        assert_eq!(
            kh.check("h", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Unknown
        );
        assert_eq!(
            kh.check("h", 2222, &key(ED_A)).unwrap(),
            HostKeyStatus::Known
        );
    }

    #[test]
    fn ipv6_literal_with_port_matches() {
        let (_d, kh, p) = setup();
        kh.add("::1", 2222, &key(ED_A)).unwrap();
        assert!(fs::read_to_string(&p).unwrap().starts_with("[::1]:2222 "));
        assert_eq!(
            kh.check("::1", 2222, &key(ED_A)).unwrap(),
            HostKeyStatus::Known
        );
        kh.add("::1", 22, &key(ED_B)).unwrap();
        assert!(
            fs::read_to_string(&p)
                .unwrap()
                .contains("\n::1 ssh-ed25519")
        );
        assert_eq!(
            kh.check("::1", 22, &key(ED_B)).unwrap(),
            HostKeyStatus::Known
        );
    }

    #[test]
    fn hashed_entry_written_by_openssh_matches() {
        let (_d, kh, p) = setup();
        fs::write(&p, format!("{HASHED_LINE}\n")).unwrap();
        assert_eq!(
            kh.check("hashed.example.com", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Known
        );
        assert_eq!(
            kh.check("hashed.example.com", 22, &key(ED_B)).unwrap(),
            HostKeyStatus::Changed {
                stored: vec![FP_ED_A.to_string()]
            }
        );
        assert_eq!(
            kh.check("other.example.com", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Unknown
        );
    }

    #[test]
    fn several_keys_per_host_are_all_listed_by_stored_algorithms() {
        let (_d, kh, _p) = setup();
        kh.add("h", 22, &key(ED_A)).unwrap();
        kh.add("h", 22, &key(RSA)).unwrap();
        kh.add("other", 22, &key(ED_B)).unwrap();
        let algs = kh.stored_algorithms("h", 22).unwrap();
        assert_eq!(algs, vec![key(ED_A).algorithm(), key(RSA).algorithm()]);
        assert!(kh.stored_algorithms("missing", 22).unwrap().is_empty());
    }

    #[test]
    fn replace_swaps_only_the_same_algorithm_and_keeps_every_other_line_byte_for_byte() {
        let (_d, kh, p) = setup();
        let mut original: Vec<u8> = b"\xEF\xBB\xBF# a comment\r\n\r\n".to_vec();
        original.extend_from_slice(format!("h ssh-ed25519 {ED_A} note\r\n").as_bytes());
        original.extend_from_slice(b"this is garbage \xFF\xFE not a key\r\n");
        original.extend_from_slice(format!("h ssh-rsa {RSA}\r\n").as_bytes());
        original.extend_from_slice(b"# trailing comment, no key here\r\n");
        original.extend_from_slice(format!("shared,h ssh-ed25519 {ED_A}\r\n").as_bytes());
        original.extend_from_slice(format!("other ssh-ed25519 {ED_A}\r\n").as_bytes());
        original.extend_from_slice(b"last line without terminator");
        fs::write(&p, &original).unwrap();

        kh.replace("h", 22, &key(ED_B)).unwrap();

        let mut expected: Vec<u8> = b"\xEF\xBB\xBF# a comment\r\n\r\n".to_vec();
        expected.extend_from_slice(b"this is garbage \xFF\xFE not a key\r\n");
        expected.extend_from_slice(format!("h ssh-rsa {RSA}\r\n").as_bytes());
        expected.extend_from_slice(b"# trailing comment, no key here\r\n");
        expected.extend_from_slice(format!("shared ssh-ed25519 {ED_A}\r\n").as_bytes());
        expected.extend_from_slice(format!("other ssh-ed25519 {ED_A}\r\n").as_bytes());
        expected.extend_from_slice(b"last line without terminator\r\n");
        expected.extend_from_slice(format!("h ssh-ed25519 {ED_B}\r\n").as_bytes());
        assert_eq!(fs::read(&p).unwrap(), expected);
        assert_eq!(fs::read(p.with_extension("bak")).unwrap(), original);
        assert_eq!(kh.check("h", 22, &key(ED_B)).unwrap(), HostKeyStatus::Known);
        assert_eq!(kh.check("h", 22, &key(RSA)).unwrap(), HostKeyStatus::Known);
    }

    #[test]
    fn garbage_and_unknown_algorithm_lines_do_not_break_lookups() {
        let (_d, kh, p) = setup();
        fs::write(
            &p,
            format!("h ssh-unknown AAAAnotakey\nh a b\n@revoked h ssh-ed25519 {ED_A}\nh ssh-ed25519 {ED_A}\n"),
        )
        .unwrap();
        assert_eq!(kh.check("h", 22, &key(ED_A)).unwrap(), HostKeyStatus::Known);
    }

    #[test]
    fn add_creates_the_file_and_leaves_bak_of_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("known_hosts");
        let kh = KnownHosts::new(&path);
        kh.add("a", 22, &key(ED_A)).unwrap();
        assert_eq!(names(path.parent().unwrap()), ["known_hosts"]);
        let first = fs::read(&path).unwrap();
        kh.add("b", 22, &key(ED_B)).unwrap();
        assert_eq!(fs::read(path.with_extension("bak")).unwrap(), first);
        assert_ne!(fs::read(&path).unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn known_hosts_is_owner_only_and_a_new_folder_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("fresh").join("fernjo");
        let path = folder.join("known_hosts");
        let kh = KnownHosts::new(&path);
        kh.add("a", 22, &key(ED_A)).unwrap();
        assert_eq!(mode(&folder), 0o700, "a folder fernjo creates is private");
        assert_eq!(mode(&path), 0o600);
        // a file an older build wrote with the default umask is tightened by the next write,
        // and so is the backup copied from it
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        kh.add("b", 22, &key(ED_B)).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&path.with_extension("bak")), 0o600);
    }

    #[test]
    fn write_is_atomic_no_temp_file_left() {
        let (d, kh, _p) = setup();
        kh.add("a", 22, &key(ED_A)).unwrap();
        kh.add("b", 22, &key(ED_B)).unwrap();
        assert_eq!(names(d.path()), ["known_hosts", "known_hosts.bak"]);
    }

    /// Host strings that must never reach the file: each would add an alias, a second line, a
    /// pattern or a marker.
    fn bad_hosts() -> Vec<String> {
        let mut v: Vec<String> = [
            "",
            "evil,victim.corp",
            "victim.corp,evil",
            "x\nvictim2 ssh-ed25519 AAAA",
            "a\rb",
            "a b",
            "a\tb",
            "a\0b",
            "a\u{7f}b",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{200b}b",
            "|1|c2FsdA==|aGFzaA==",
            "@cert-authority",
            "@revoked",
            "!victim.corp",
            "#comment",
            "*.corp",
            "*",
            "h?",
            "\"h\"",
            "'h'",
            "[h]",
            "[::1]:22",
            "h]:22",
            "a/b",
            "user@host",
            "host:22",
            "1:2",
            "::1::2",
            "fe80::1%",
            "fe80::1%a b",
            "fe80::1%a,b",
            "%eth0",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        v.push("a".repeat(256));
        v
    }

    fn good_hosts() -> Vec<String> {
        let mut v: Vec<String> = [
            "host.example",
            "HOST.Example",
            "localhost",
            "a",
            "10.0.0.1",
            "my_host.lab",
            "host.example.",
            "web-01.lab",
            "xn--bcher-kva.example",
            "::1",
            "2001:db8::1",
            "fe80::1%eth0",
            "fe80::1%12",
            "::ffff:192.0.2.1",
            "b\u{fc}cher.example",
            "\u{4f8b}\u{3048}.jp",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        v.push("a".repeat(255));
        v
    }

    #[test]
    fn the_writer_refuses_hosts_that_cannot_be_stored_safely_and_leaves_the_file_alone() {
        let (_d, kh, p) = setup();
        let original = format!("victim.corp ssh-ed25519 {ED_B}\n");
        fs::write(&p, &original).unwrap();
        for host in bad_hosts() {
            for port in [22, 2222] {
                let label = format!("{host:?}:{port}");
                assert!(
                    matches!(kh.add(&host, port, &key(ED_A)), Err(SshError::InvalidHost)),
                    "add {label}"
                );
                assert!(
                    matches!(
                        kh.replace(&host, port, &key(ED_A)),
                        Err(SshError::InvalidHost)
                    ),
                    "replace {label}"
                );
                assert!(
                    matches!(
                        kh.check(&host, port, &key(ED_A)),
                        Err(SshError::InvalidHost)
                    ),
                    "check {label}"
                );
                assert!(
                    matches!(
                        kh.stored_algorithms(&host, port),
                        Err(SshError::InvalidHost)
                    ),
                    "stored_algorithms {label}"
                );
            }
            assert_eq!(fs::read_to_string(&p).unwrap(), original, "{host:?}");
        }
        // the probe of the review: no alias for the victim, no second line
        assert!(kh.add("evil,victim.corp", 22, &key(ED_A)).is_err());
        assert_eq!(
            kh.check("victim.corp", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Changed {
                stored: vec![FP_ED_B.to_string()]
            },
            "the victim keeps only its own key"
        );
    }

    #[test]
    fn legitimate_hosts_round_trip_and_match_only_themselves() {
        for host in good_hosts() {
            for port in [22, 2222] {
                let (_d, kh, p) = setup();
                kh.add(&host, port, &key(ED_A)).unwrap();
                let text = fs::read_to_string(&p).unwrap();
                assert_eq!(text.lines().count(), 1, "{host:?}");
                assert_eq!(
                    kh.check(&host, port, &key(ED_A)).unwrap(),
                    HostKeyStatus::Known,
                    "{host:?}:{port}"
                );
                assert_eq!(
                    kh.check(&host.to_ascii_uppercase(), port, &key(ED_A))
                        .unwrap(),
                    HostKeyStatus::Known,
                    "case-insensitive {host:?}:{port}"
                );
                assert_eq!(
                    kh.check("other.example", port, &key(ED_A)).unwrap(),
                    HostKeyStatus::Unknown,
                    "{host:?}:{port}"
                );
                let other_port = if port == 22 { 2222 } else { 22 };
                assert_eq!(
                    kh.check(&host, other_port, &key(ED_A)).unwrap(),
                    HostKeyStatus::Unknown,
                    "port {host:?}:{port}"
                );
                // replace works for a legitimate host too
                kh.replace(&host, port, &key(ED_B)).unwrap();
                assert_eq!(
                    kh.check(&host, port, &key(ED_B)).unwrap(),
                    HostKeyStatus::Known
                );
                assert_eq!(
                    kh.check(&host, port, &key(ED_A)).unwrap(),
                    HostKeyStatus::Changed {
                        stored: vec![FP_ED_B.to_string()]
                    }
                );
            }
        }
    }

    #[test]
    fn validate_host_accepts_the_good_table_and_refuses_the_bad_one() {
        for host in good_hosts() {
            assert!(validate_host(&host).is_ok(), "{host:?}");
        }
        for host in bad_hosts() {
            assert!(
                matches!(validate_host(&host), Err(SshError::InvalidHost)),
                "{host:?}"
            );
        }
    }

    #[test]
    fn replace_removes_every_matching_entry_of_a_comma_list_line() {
        let (_d, kh, p) = setup();
        // the same host twice on one hand-edited line, next to another host
        fs::write(
            &p,
            format!("h,other,H ssh-ed25519 {ED_A}\nh,H ssh-ed25519 {ED_A}\n"),
        )
        .unwrap();
        kh.replace("h", 22, &key(ED_B)).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert_eq!(
            text,
            format!("other ssh-ed25519 {ED_A}\nh ssh-ed25519 {ED_B}\n")
        );
        assert_eq!(kh.check("h", 22, &key(ED_B)).unwrap(), HostKeyStatus::Known);
        assert_eq!(
            kh.check("other", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Known,
            "the other host keeps its trust"
        );
    }

    #[test]
    fn trust_always_replaces_a_different_key_of_the_same_type() {
        let (_d, kh, p) = setup();
        kh.add("h", 22, &key(ED_A)).unwrap();
        kh.add("h", 22, &key(RSA)).unwrap();
        kh.trust("h", 22, &key(ED_B)).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.contains(ED_A), "{text}");
        assert_eq!(kh.check("h", 22, &key(ED_B)).unwrap(), HostKeyStatus::Known);
        assert_eq!(kh.check("h", 22, &key(RSA)).unwrap(), HostKeyStatus::Known);
        // nothing stored yet: same as add
        kh.trust("new", 22, &key(ED_A)).unwrap();
        assert_eq!(
            kh.check("new", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Known
        );
    }

    #[test]
    fn concurrent_confirmations_do_not_lose_an_update() {
        let (_d, kh, _p) = setup();
        let hosts: Vec<String> = (0..40).map(|i| format!("host{i}.example")).collect();
        std::thread::scope(|scope| {
            for half in hosts.chunks(20) {
                let kh = kh.clone();
                scope.spawn(move || {
                    for h in half {
                        kh.add(h, 22, &key(ED_A)).unwrap();
                    }
                });
            }
        });
        for h in &hosts {
            assert_eq!(
                kh.check(h, 22, &key(ED_A)).unwrap(),
                HostKeyStatus::Known,
                "{h}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn temp_file_is_never_a_fixed_name() {
        let (d, kh, p) = setup();
        let victim = d.path().join("victim");
        fs::write(&victim, "untouched").unwrap();
        // the names the old code used
        std::os::unix::fs::symlink(&victim, d.path().join("known_hosts.tmp")).unwrap();
        kh.add("a", 22, &key(ED_A)).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "untouched");
        assert!(fs::read_to_string(&p).unwrap().contains(ED_A));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_at_the_known_hosts_bak_is_not_followed() {
        let (d, kh, p) = setup();
        let victim = d.path().join("victim");
        fs::write(&victim, "untouched").unwrap();
        std::os::unix::fs::symlink(&victim, d.path().join("known_hosts.bak")).unwrap();
        kh.add("a", 22, &key(ED_A)).unwrap();
        let first = fs::read(&p).unwrap();
        kh.add("b", 22, &key(ED_B)).unwrap();
        assert_eq!(fs::read_to_string(&victim).unwrap(), "untouched");
        let bak = d.path().join("known_hosts.bak");
        assert!(fs::symlink_metadata(&bak).unwrap().file_type().is_file());
        assert_eq!(fs::read(&bak).unwrap(), first);
    }

    #[test]
    fn write_never_uses_the_old_fixed_temp_name() {
        let (d, kh, _p) = setup();
        // a stale file at the old fixed name is neither used nor removed
        fs::write(d.path().join("known_hosts.tmp"), "stale").unwrap();
        kh.add("a", 22, &key(ED_A)).unwrap();
        assert_eq!(
            fs::read_to_string(d.path().join("known_hosts.tmp")).unwrap(),
            "stale"
        );
    }

    #[test]
    fn an_oversized_known_hosts_is_refused_and_left_alone() {
        let (d, kh, p) = setup();
        // a real line first, so a reader without the cap would find the host
        let line = format!("big.example ssh-ed25519 {ED_A}\n");
        fs::write(&p, &line).unwrap();
        let file = fs::OpenOptions::new().write(true).open(&p).unwrap();
        file.set_len(MAX_KNOWN_HOSTS_LEN + 1).unwrap();
        drop(file);

        expect_io(kh.check("big.example", 22, &key(ED_A)));
        expect_io(kh.stored_algorithms("big.example", 22));
        expect_io(kh.add("a", 22, &key(ED_B)));
        assert_eq!(fs::metadata(&p).unwrap().len(), MAX_KNOWN_HOSTS_LEN + 1);
        assert_eq!(names(d.path()), ["known_hosts"], "nothing written");

        // exactly at the cap is still read
        let file = fs::OpenOptions::new().write(true).open(&p).unwrap();
        file.set_len(MAX_KNOWN_HOSTS_LEN).unwrap();
        drop(file);
        assert_eq!(
            kh.check("big.example", 22, &key(ED_A)).unwrap(),
            HostKeyStatus::Known
        );
    }

    #[test]
    fn a_folder_at_the_known_hosts_name_is_an_error_not_an_empty_file() {
        let (d, kh, p) = setup();
        fs::create_dir(&p).unwrap();
        expect_io(kh.check("h", 22, &key(ED_A)));
        expect_io(kh.add("h", 22, &key(ED_A)));
        assert!(p.is_dir());
        assert_eq!(names(d.path()), ["known_hosts"]);
    }

    /// Every regular file below `dir` (recursively) that contains `needle`.
    #[cfg(unix)]
    fn files_containing(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                found.extend(files_containing(&path, needle));
            } else if meta.file_type().is_file()
                && fs::read(&path)
                    .unwrap()
                    .windows(needle.len())
                    .any(|w| w == needle)
            {
                found.push(path);
            }
        }
        found
    }

    #[cfg(unix)]
    #[test]
    fn add_over_a_symlinked_known_hosts_does_not_copy_the_link_target() {
        let data = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("id_ed25519");
        let sentinel = b"-----BEGIN OPENSSH PRIVATE KEY----- SENTINEL";
        fs::write(&secret, sentinel).unwrap();
        let p = data.path().join("known_hosts");
        std::os::unix::fs::symlink(&secret, &p).unwrap();
        let kh = KnownHosts::new(&p);

        expect_io(kh.check("h", 22, &key(ED_A)));
        expect_io(kh.add("h", 22, &key(ED_A)));
        expect_io(kh.replace("h", 22, &key(ED_A)));

        assert!(files_containing(data.path(), sentinel).is_empty());
        assert_eq!(fs::read(&secret).unwrap(), sentinel);
        assert!(fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
        assert_eq!(names(data.path()), ["known_hosts"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_at_the_known_hosts_name_is_refused_without_blocking() {
        let (d, _kh, p) = setup();
        let status = std::process::Command::new("mkfifo")
            .arg(&p)
            .status()
            .unwrap();
        assert!(status.success());
        let (tx, rx) = std::sync::mpsc::channel();
        let path = p.clone();
        std::thread::spawn(move || {
            let kh = KnownHosts::new(&path);
            let _ = tx.send((kh.check("h", 22, &key(ED_A)), kh.add("h", 22, &key(ED_A))));
        });
        let (check, add) = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("opening the FIFO blocked");
        expect_io(check);
        expect_io(add);
        assert_eq!(names(d.path()), ["known_hosts"]);
    }
}
