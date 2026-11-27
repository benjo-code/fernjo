//! Pure update decisions: which version is offered, how release notes are cleaned, where the
//! release page is and whether this install can update itself. No I/O; the caller injects it.

use std::path::{Path, PathBuf};

use semver::Version;

pub const MAX_UPDATE_BYTES: u64 = 256 * 1024 * 1024;
pub const MAX_NOTES_BYTES: usize = 16 * 1024;
/// The generic releases page, opened when no version is known.
pub const RELEASES_LATEST_URL: &str = "https://github.com/benjo-code/fernjo/releases/latest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(clippy::enum_variant_names, reason = "MacOs is the platform's name")]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

#[derive(Debug, Clone)]
pub struct InstallEnv {
    pub os: Os,
    pub appimage: Option<PathBuf>,
    pub exe: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleasePageReason {
    PackageInstall,
    NotWritable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    SelfUpdate,
    ReleasePageOnly(ReleasePageReason),
}

/// A candidate is offered when it is a plain release (no pre-release, no build metadata),
/// strictly newer than `current` and not the version the user chose to skip.
pub fn is_offered(current: &str, candidate: &str, skipped: Option<&str>) -> bool {
    let (Ok(current), Ok(candidate)) = (Version::parse(current), Version::parse(candidate)) else {
        return false;
    };
    if !candidate.pre.is_empty() || !candidate.build.is_empty() || candidate <= current {
        return false;
    }
    match skipped {
        Some(s) => match Version::parse(s) {
            Ok(v) => v != candidate,
            Err(_) => s != candidate.to_string(),
        },
        None => true,
    }
}

/// Release notes as plain text: control characters removed (tab and newline kept, CRLF becomes
/// LF, a lone CR is dropped) and cut to `MAX_NOTES_BYTES` on a char boundary. The text is never
/// interpreted; escaping is the UI's job. Unicode format characters (bidi controls, zero-width
/// characters) are not control characters and pass through; the text is for display only.
pub fn clean_notes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_NOTES_BYTES));
    for c in raw.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_NOTES_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

/// The GitHub release page of a plain `MAJOR.MINOR.PATCH` version, `None` for anything else.
pub fn release_page_url(version: &str) -> Option<String> {
    let parts: Vec<&str> = version.split('.').collect();
    let plain = parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && (*p == "0" || !p.starts_with('0'))
        });
    plain.then(|| format!("https://github.com/benjo-code/fernjo/releases/tag/v{version}"))
}

pub fn install_kind(env: &InstallEnv, writable: &dyn Fn(&Path) -> bool) -> InstallKind {
    let not_writable = InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable);
    match env.os {
        Os::Windows => InstallKind::SelfUpdate,
        Os::MacOs => {
            // .../fernjo.app/Contents/MacOS/fernjo: the folder that holds the bundle must be
            // writable so the bundle can be replaced.
            let bundle = env
                .exe
                .ancestors()
                .find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("app")));
            match bundle.and_then(Path::parent) {
                Some(dir) if writable(dir) => InstallKind::SelfUpdate,
                _ => not_writable,
            }
        }
        Os::Linux => match env.appimage.as_deref() {
            None => InstallKind::ReleasePageOnly(ReleasePageReason::PackageInstall),
            Some(img) => match img.parent() {
                Some(dir) if writable(dir) => InstallKind::SelfUpdate,
                _ => not_writable,
            },
        },
    }
}

/// Whether a download stays within `cap` bytes: the declared length (when the server sent one)
/// and the bytes received so far. Production passes `MAX_UPDATE_BYTES`.
pub fn within_download_cap(cap: u64, declared: Option<u64>, received: u64) -> bool {
    declared.is_none_or(|d| d <= cap) && received <= cap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_strictly_newer_stable_only() {
        assert!(is_offered("0.4.0", "0.4.1", None));
        assert!(!is_offered("0.4.0", "0.4.0", None));
        assert!(!is_offered("0.4.1", "0.4.0", None));
        assert!(!is_offered("0.4.0", "0.4.1-rc.1", None));
        assert!(!is_offered("0.4.0", "0.4.1+build5", None));
        assert!(!is_offered("0.4.0", "garbage", None));
        assert!(!is_offered("0.4.0", "", None));
        assert!(!is_offered("garbage", "0.4.1", None));
    }

    #[test]
    fn skipped_version_is_not_offered_but_newer_is() {
        assert!(!is_offered("0.4.0", "0.4.1", Some("0.4.1")));
        assert!(is_offered("0.4.0", "0.4.2", Some("0.4.1")));
        assert!(is_offered("0.4.0", "0.4.1", Some("not a version")));
        // a mangled skip value never hides an update
        assert!(is_offered("0.4.0", "0.4.1", Some("0.4.01")));
        assert!(is_offered("0.4.0", "0.4.1", Some("0.4.1+x")));
    }

    #[test]
    fn notes_are_cut_at_16_kib_on_a_char_boundary() {
        let mut s = "a".repeat(MAX_NOTES_BYTES - 1);
        s.push('€'); // 3 bytes, straddles the limit
        s.push_str("tail");
        let out = clean_notes(&s);
        assert_eq!(out.len(), MAX_NOTES_BYTES - 1);
        assert!(out.chars().all(|c| c == 'a'));
        assert!(clean_notes(&"b".repeat(100)).len() == 100);
    }

    #[test]
    fn notes_lose_control_characters_but_keep_tab_and_newline() {
        assert_eq!(
            clean_notes("a\u{1b}[31mb\u{0}\tc\nd\r\ne\rf\u{7f}"),
            "a[31mb\tc\nd\nef"
        );
    }

    #[test]
    fn notes_keep_markup_as_literal_text() {
        assert_eq!(clean_notes("<script>x</script>"), "<script>x</script>");
    }

    #[test]
    fn release_page_url_only_for_plain_versions() {
        assert_eq!(
            release_page_url("0.4.1").as_deref(),
            Some("https://github.com/benjo-code/fernjo/releases/tag/v0.4.1")
        );
        for bad in [
            "",
            "0.4",
            "0.4.1-rc.1",
            "0.4.1+b",
            "v0.4.1",
            "0.4.x",
            "../1.2.3",
            "1.2.3/x",
            "01.2.3",
        ] {
            assert_eq!(release_page_url(bad), None, "{bad}");
        }
    }

    fn env(os: Os, appimage: Option<&str>, exe: &str) -> InstallEnv {
        InstallEnv {
            os,
            appimage: appimage.map(PathBuf::from),
            exe: PathBuf::from(exe),
        }
    }

    #[test]
    fn install_kind_table() {
        let yes = |_: &Path| true;
        let no = |_: &Path| false;
        let win = env(Os::Windows, None, r"C:\Apps\fernjo.exe");
        assert_eq!(install_kind(&win, &no), InstallKind::SelfUpdate);

        let mac = env(
            Os::MacOs,
            None,
            "/Applications/fernjo.app/Contents/MacOS/fernjo",
        );
        assert_eq!(install_kind(&mac, &yes), InstallKind::SelfUpdate);
        assert_eq!(
            install_kind(&mac, &no),
            InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable)
        );
        let seen = |p: &Path| p == Path::new("/Applications");
        assert_eq!(install_kind(&mac, &seen), InstallKind::SelfUpdate);
        let bare = env(Os::MacOs, None, "/usr/local/bin/fernjo");
        assert_eq!(
            install_kind(&bare, &yes),
            InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable)
        );

        let lin_pkg = env(Os::Linux, None, "/usr/bin/fernjo");
        assert_eq!(
            install_kind(&lin_pkg, &yes),
            InstallKind::ReleasePageOnly(ReleasePageReason::PackageInstall)
        );
        let lin = env(
            Os::Linux,
            Some("/home/u/Apps/fernjo.AppImage"),
            "/tmp/.mount_x/usr/bin/fernjo",
        );
        assert_eq!(install_kind(&lin, &yes), InstallKind::SelfUpdate);
        assert_eq!(
            install_kind(&lin, &no),
            InstallKind::ReleasePageOnly(ReleasePageReason::NotWritable)
        );
        let dir = |p: &Path| p == Path::new("/home/u/Apps");
        assert_eq!(install_kind(&lin, &dir), InstallKind::SelfUpdate);
    }

    #[test]
    fn download_cap_uses_declared_and_received() {
        let cap = MAX_UPDATE_BYTES;
        assert!(within_download_cap(cap, None, 0));
        assert!(within_download_cap(cap, Some(cap), cap));
        assert!(!within_download_cap(cap, Some(cap + 1), 0));
        assert!(!within_download_cap(cap, None, cap + 1));
        assert!(!within_download_cap(cap, Some(1), cap + 1));
        assert!(within_download_cap(10, Some(10), 10));
        assert!(!within_download_cap(10, Some(11), 0));
    }
}
