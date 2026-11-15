//! Detects copies a sync tool leaves behind after a conflict.

use std::path::Path;

const MARKERS: [&str; 3] = [
    "sync-conflict",   // Syncthing
    "conflicted copy", // Dropbox
    "_conflict-",      // Nextcloud
];

/// Names of the files and folders directly inside `dir` that look like sync-conflict copies,
/// sorted. Matching is case-insensitive on the name only; nothing is read or recursed into.
/// A missing or unreadable `dir` yields an empty list.
pub fn find_conflicts(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type()
                .map(|t| t.is_file() || t.is_dir())
                .unwrap_or(false)
        })
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            let lower = name.to_lowercase();
            MARKERS.iter().any(|m| lower.contains(m))
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_syncthing_dropbox_and_nextcloud_copies_sorted() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "vault.fernjo.sync-conflict-20260101-120000-ABC.fernjo",
            "connections (Ben's conflicted copy 2026-01-01).json",
            "connections_conflict-20260101-1.json",
            "CONNECTIONS.SYNC-CONFLICT-X.json",
        ] {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        fs::create_dir(dir.path().join("folder.sync-conflict-1")).unwrap();
        fs::create_dir(dir.path().join("nested")).unwrap();
        fs::write(
            dir.path().join("nested").join("a.sync-conflict-1.json"),
            "x",
        )
        .unwrap();
        let found = find_conflicts(dir.path());
        let mut expected = vec![
            "CONNECTIONS.SYNC-CONFLICT-X.json".to_string(),
            "connections (Ben's conflicted copy 2026-01-01).json".to_string(),
            "connections_conflict-20260101-1.json".to_string(),
            "folder.sync-conflict-1".to_string(),
            "vault.fernjo.sync-conflict-20260101-120000-ABC.fernjo".to_string(),
        ];
        expected.sort();
        assert_eq!(found, expected);
    }

    #[test]
    fn ignores_normal_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["connections.json", "connections.json.bak", "vault.fernjo"] {
            fs::write(dir.path().join(name), "x").unwrap();
        }
        assert!(find_conflicts(dir.path()).is_empty());
    }

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_conflicts(&dir.path().join("nope")).is_empty());
    }
}
