//! OpenSSH-compatible `known_hosts` file. The file is shared with OpenSSH, so a rewrite only ever
//! touches the lines it owns; every other byte (comments, blank or garbage lines, CRLF, BOM) stays.
//!
//! Matching is done locally instead of through `russh::keys::known_hosts::known_host_keys_path`:
//! that helper fails the whole lookup on one unparsable key line, does not count comment lines in
//! the line numbers it reports, and mishandles CRLF and a leading BOM. Supported host patterns are
//! exact names (`host`, `[host]:port`) and hashed `|1|` entries; wildcards, negations and `@marker`
//! lines are left alone and never match.

use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use data_encoding::BASE64;
use hmac::{Hmac, KeyInit, Mac};
use russh::keys::ssh_key::HashAlg;
use russh::keys::{Algorithm, PublicKey, parse_public_key_base64};
use sha1::Sha1;

use crate::SshError;

const BOM: &[u8] = b"\xEF\xBB\xBF";

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

/// Handle on one `known_hosts` file. A missing file counts as empty.
#[derive(Debug, Clone)]
pub struct KnownHosts {
    path: PathBuf,
}

/// One line that matches a host: where it lives in the file and what it stores.
struct Match {
    /// The whole line including its terminator.
    line: Range<usize>,
    /// The host-pattern field.
    hosts: Range<usize>,
    /// Index of the matching entry within the comma-separated host field.
    entry: usize,
    key: PublicKey,
}

impl KnownHosts {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn check(&self, host: &str, port: u16, key: &PublicKey) -> Result<HostKeyStatus, SshError> {
        let bytes = self.read()?;
        let same: Vec<PublicKey> = find_matches(&bytes, &host_port(host, port))
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
        for m in find_matches(&bytes, &host_port(host, port)) {
            let alg = m.key.algorithm();
            if !out.contains(&alg) {
                out.push(alg);
            }
        }
        Ok(out)
    }

    /// Appends the key unless it is already stored for the host and port.
    pub fn add(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        let mut bytes = self.read()?;
        let name = host_port(host, port);
        if find_matches(&bytes, &name).iter().any(|m| &m.key == key) {
            return Ok(());
        }
        append_line(&mut bytes, &name, key)?;
        write_atomic(&self.path, &bytes)
    }

    /// Drops the stored keys of the same algorithm for the host and port, then appends the new one.
    pub fn replace(&self, host: &str, port: u16, key: &PublicKey) -> Result<(), SshError> {
        let bytes = self.read()?;
        let name = host_port(host, port);
        let drop_these: Vec<Match> = find_matches(&bytes, &name)
            .into_iter()
            .filter(|m| m.key.algorithm() == key.algorithm())
            .collect();
        let mut out = Vec::with_capacity(bytes.len() + 128);
        let mut pos = 0;
        // Matches come in file order and do not overlap.
        for m in &drop_these {
            out.extend_from_slice(&bytes[pos..m.line.start]);
            let entries: Vec<&[u8]> = bytes[m.hosts.clone()].split(|&b| b == b',').collect();
            // A line shared with other hosts loses only this host's entry; a line that belongs to
            // this host alone is dropped entirely.
            if entries.len() > 1 {
                out.extend_from_slice(&bytes[m.line.start..m.hosts.start]);
                let kept: Vec<&[u8]> = entries
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != m.entry)
                    .map(|(_, e)| *e)
                    .collect();
                out.extend_from_slice(&kept.join(&b","[..]));
                out.extend_from_slice(&bytes[m.hosts.end..m.line.end]);
            }
            pos = m.line.end;
        }
        out.extend_from_slice(&bytes[pos..]);
        append_line(&mut out, &name, key)?;
        write_atomic(&self.path, &out)
    }

    fn read(&self) -> Result<Vec<u8>, SshError> {
        match fs::read(&self.path) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(SshError::Io),
        }
    }
}

/// The host name as OpenSSH writes it: bare on port 22, `[host]:port` otherwise. Lowercased.
fn host_port(host: &str, port: u16) -> String {
    let host = host.to_ascii_lowercase();
    if port == 22 {
        host
    } else {
        format!("[{host}]:{port}")
    }
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
    let entry = bytes[hosts.clone()]
        .split(|&b| b == b',')
        .position(|e| entry_matches(e, name))?;
    let key_b64 = std::str::from_utf8(&content[fields[2].clone()]).ok()?;
    let key = parse_public_key_base64(key_b64).ok()?;
    Some(Match {
        line,
        hosts,
        entry,
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

/// Creates (truncating) `path` readable and writable by its owner only on Unix (mode 0600, also
/// when a stale file of that name already exists); elsewhere the folder's inherited ACL applies.
fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    owner_only(path)?;
    Ok(file)
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

/// Temp file next to `path`, `sync_all`, current file copied to `.bak`, rename over `path`.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<(), SshError> {
    let with_suffix = |suffix: &str| {
        let mut s = path.as_os_str().to_owned();
        s.push(suffix);
        PathBuf::from(s)
    };
    let tmp = with_suffix(".tmp");
    let result = (|| -> io::Result<()> {
        if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            create_private_dir_all(dir)?;
        }
        let mut file = create_private_file(&tmp)?;
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            let bak = with_suffix(".bak");
            fs::copy(path, &bak)?;
            owner_only(&bak)?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(SshError::Io);
    }
    Ok(())
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
}
