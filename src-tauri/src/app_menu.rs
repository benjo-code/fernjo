//! Offers to add fernjo to the desktop's application menu when it runs as an AppImage (Linux).
//!
//! Under Wayland the title bar and task bar icon of a window is not set by the program: the
//! compositor looks the window's app id (`fernjo`) up in the installed `.desktop` entries. A `.deb`
//! installs one; an AppImage that nothing integrated has none, so the window shows a placeholder.
//! `Window::set_icon` does not help there (GTK 3 has no way to send an icon to a Wayland
//! compositor). The only fix is the entry, which this module can write after the user agreed. It
//! never writes anything without the answer "Add", and it only runs for an AppImage (`APPIMAGE`
//! names the file) outside scratch runs (`FERNJO_DATA_DIR` set).
//!
//! The pure parts (entry text, paths, the decision) compile and are tested on every OS; only the
//! call in `lib.rs` is Linux-only.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};

use fernjo_fs::{FileMode, Parent, WriteOptions};

/// The app id the window gets (the program name), so the entry must be called `fernjo.desktop`.
const ENTRY_NAME: &str = "fernjo.desktop";
const ICON_NAME: &str = "fernjo.png";
const ICON_BYTES: &[u8] = include_bytes!("../icons/128x128.png");
/// The largest existing menu entry that is compared with the one fernjo would write (the real one
/// is well under 1 KiB); a larger file, like a missing or unreadable one, is offered again.
const MAX_ENTRY_LEN: u64 = 64 * 1024;
/// Marker in the device-local folder: the user chose "Never".
pub const NEVER_FILE: &str = "appmenu-never";

/// The user's answer to the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Add,
    Never,
    NotNow,
}

/// Where the two files go below `data_home` (`$XDG_DATA_HOME`, usually `~/.local/share`).
pub fn entry_paths(data_home: &Path) -> (PathBuf, PathBuf) {
    (
        data_home.join("applications").join(ENTRY_NAME),
        data_home
            .join("icons")
            .join("hicolor")
            .join("128x128")
            .join("apps")
            .join(ICON_NAME),
    )
}

/// `$XDG_DATA_HOME` when it is an absolute path, else `<home>/.local/share`.
pub fn data_home(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    // a plain '/' test, not `Path::is_absolute`: this is the Linux rule, and the tests also run
    // on Windows, where '/x' has no drive and counts as relative
    match xdg {
        Some(x) if x.starts_with('/') => Some(PathBuf::from(x)),
        _ => home
            .filter(|h| h.starts_with('/'))
            .map(|h| Path::new(h).join(".local/share")),
    }
}

/// The `Exec` value for `path`: quoted as the Desktop Entry specification wants (reserved characters
/// escaped inside double quotes, then every backslash doubled for the string syntax of the file).
/// `None` for a path that cannot be written safely (a line break or a NUL).
pub fn quote_exec(path: &str) -> Option<String> {
    if path.chars().any(|c| c == '\n' || c == '\r' || c == '\0') {
        return None;
    }
    let mut quoted = String::from("\"");
    for c in path.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                quoted.push('\\');
                quoted.push(c);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(c),
        }
    }
    quoted.push('"');
    Some(quoted.replace('\\', "\\\\"))
}

/// The text of `fernjo.desktop` for an AppImage at `appimage`.
pub fn desktop_entry(appimage: &str) -> Option<String> {
    let exec = quote_exec(appimage)?;
    Some(format!(
        "[Desktop Entry]\nType=Application\nName=fernjo\nComment=SSH connection manager with a \
         connection tree and tabbed sessions.\nExec={exec}\nIcon=fernjo\nStartupWMClass=fernjo\n\
         Categories=Development;\nTerminal=false\n"
    ))
}

/// Whether to show the dialog: only for an AppImage with an absolute path, never after "Never",
/// and not when the entry already says exactly what it would say now (an entry that points to a
/// moved file is asked about again). An entry that is a symbolic link (checked with
/// `symlink_metadata`, never followed) is managed by the user, for example by a dotfile manager:
/// no dialog, since "Add" could not replace it anyway (`write_atomic` refuses a non-regular
/// target). Otherwise the entry is read through `fernjo_fs::read_capped` (a regular file of at
/// most `MAX_ENTRY_LEN`); any read failure means "no current entry".
pub fn should_offer(appimage: Option<&str>, data_home: &Path, never: bool) -> bool {
    // a plain '/' test like `data_home`: a relative path would resolve against whatever the
    // working directory is when the desktop starts the entry
    let Some(appimage) = appimage.filter(|a| a.starts_with('/')) else {
        return false;
    };
    if never {
        return false;
    }
    let Some(wanted) = desktop_entry(appimage) else {
        return false;
    };
    let (entry, _) = entry_paths(data_home);
    if std::fs::symlink_metadata(&entry).is_ok_and(|m| m.file_type().is_symlink()) {
        return false;
    }
    fernjo_fs::read_capped(&entry, MAX_ENTRY_LEN).map_or(true, |have| have != wanted.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // default folder and file modes (the menu entry is meant to be read by the desktop); the
    // temp name is random and never reused, so nothing planted at a fixed name matters
    fernjo_fs::write_atomic(
        path,
        bytes,
        WriteOptions {
            parent: Parent::CreateDefault,
            backup: false,
            file_mode: FileMode::Default,
        },
    )
}

/// Writes the icon and the entry (icon first, so the entry never names a missing icon).
pub fn install(appimage: &str, data_home: &Path) -> std::io::Result<()> {
    let entry = desktop_entry(appimage)
        .ok_or_else(|| std::io::Error::other("the AppImage path cannot be written to an entry"))?;
    let (entry_path, icon_path) = entry_paths(data_home);
    write_atomic(&icon_path, ICON_BYTES)?;
    write_atomic(&entry_path, entry.as_bytes())
}

/// Remembers "Never" in the device-local folder.
pub fn remember_never(device_dir: &Path) -> std::io::Result<()> {
    write_atomic(&device_dir.join(NEVER_FILE), b"never\n")
}

/// Whether the user chose "Never" on this device.
pub fn is_never(device_dir: Option<&Path>) -> bool {
    device_dir.is_some_and(|d| d.join(NEVER_FILE).exists())
}

/// The text of the dialog that asks to add fernjo to the application menu.
pub fn offer_message(data_home: &Path, appimage: &str) -> String {
    format!(
        "fernjo runs as an AppImage, so your desktop has no application menu entry for it \
         and shows a placeholder icon in the title bar.\n\n\
         Add fernjo to your application menu? This writes two small files below {} and points \
         the entry at\n{appimage}\n\n\
         The icon may appear in the start menu and dock only after you log out and in again.\n\n\
         If you want to move the AppImage first, choose \"Not now\"; the question comes back \
         at the next start.",
        data_home.display()
    )
}

/// Shows the dialog when it is due (Linux AppImage, not a scratch run). Never blocks: the answer is
/// handled in the dialog callback, and a failed write is ignored (the entry is a convenience).
#[cfg(target_os = "linux")]
pub fn offer(app: &tauri::AppHandle) {
    use tauri::Manager as _;
    use tauri_plugin_dialog::{
        DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
    };

    if std::env::var_os("FERNJO_DATA_DIR").is_some() {
        return;
    }
    let appimage = std::env::var("APPIMAGE").ok();
    let xdg = std::env::var("XDG_DATA_HOME").ok();
    let home = std::env::var("HOME").ok();
    let Some(data_home) = data_home(xdg.as_deref(), home.as_deref()) else {
        return;
    };
    let device_dir = app.state::<crate::AppState>().device_dir.clone();
    if !should_offer(
        appimage.as_deref(),
        &data_home,
        is_never(device_dir.as_deref()),
    ) {
        return;
    }
    let Some(appimage) = appimage else { return };
    app.dialog()
        .message(offer_message(&data_home, &appimage))
        .title("Add fernjo to the application menu?")
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::YesNoCancelCustom(
            "Add".into(),
            "Never".into(),
            "Not now".into(),
        ))
        .show_with_result(move |result| {
            let answer = match result {
                MessageDialogResult::Yes => Answer::Add,
                MessageDialogResult::No => Answer::Never,
                MessageDialogResult::Custom(label) => match label.as_str() {
                    "Add" => Answer::Add,
                    "Never" => Answer::Never,
                    _ => Answer::NotNow,
                },
                _ => Answer::NotNow,
            };
            match answer {
                Answer::Add => {
                    let _ = install(&appimage, &data_home);
                }
                Answer::Never => {
                    if let Some(dir) = device_dir {
                        let _ = remember_never(&dir);
                    }
                }
                Answer::NotNow => {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "/home/u/Apps/fernjo_0.1.1_amd64.AppImage";

    #[test]
    fn data_home_prefers_an_absolute_xdg_value() {
        assert_eq!(
            data_home(Some("/x/share"), Some("/home/u")),
            Some(PathBuf::from("/x/share"))
        );
        assert_eq!(
            data_home(Some("relative"), Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share"))
        );
        assert_eq!(
            data_home(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share"))
        );
        assert_eq!(data_home(None, Some("rel")), None);
        assert_eq!(data_home(None, None), None);
    }

    #[test]
    fn the_entry_is_called_fernjo_desktop_and_the_icon_lives_in_hicolor() {
        let (entry, icon) = entry_paths(Path::new("/d"));
        assert_eq!(entry, PathBuf::from("/d/applications/fernjo.desktop"));
        assert_eq!(
            icon,
            PathBuf::from("/d/icons/hicolor/128x128/apps/fernjo.png")
        );
    }

    #[test]
    fn exec_is_quoted_and_escaped_for_the_desktop_entry_format() {
        assert_eq!(quote_exec(APP).as_deref(), Some(&*format!("\"{APP}\"")));
        assert_eq!(
            quote_exec("/a b/x.AppImage").as_deref(),
            Some("\"/a b/x.AppImage\"")
        );
        // `"`, a backtick and `$` get a backslash in the quoted string, and every backslash is
        // doubled again for the file's string syntax
        assert_eq!(quote_exec("/a\"b").as_deref(), Some("\"/a\\\\\"b\""));
        assert_eq!(quote_exec("/a$b`c").as_deref(), Some("\"/a\\\\$b\\\\`c\""));
        assert_eq!(quote_exec("/a\\b").as_deref(), Some("\"/a\\\\\\\\b\""));
        // `%` would start a field code
        assert_eq!(quote_exec("/100%").as_deref(), Some("\"/100%%\""));
        assert_eq!(quote_exec("/a\nb"), None);
        assert_eq!(quote_exec("/a\0b"), None);
    }

    #[test]
    fn the_entry_carries_the_wm_class_the_window_reports() {
        let text = desktop_entry(APP).unwrap();
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("\nStartupWMClass=fernjo\n"));
        assert!(text.contains("\nIcon=fernjo\n"));
        assert!(text.contains(&format!("\nExec=\"{APP}\"\n")));
        assert!(text.ends_with('\n'));
        assert_eq!(desktop_entry("/a\nb"), None);
    }

    #[test]
    fn it_offers_only_for_an_appimage_that_has_no_matching_entry_and_no_never() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        assert!(!should_offer(None, home, false), "not an AppImage");
        assert!(!should_offer(Some(""), home, false), "empty APPIMAGE");
        assert!(!should_offer(Some(APP), home, true), "never");
        assert!(should_offer(Some(APP), home, false), "no entry yet");
        install(APP, home).unwrap();
        assert!(!should_offer(Some(APP), home, false), "entry is current");
        assert!(
            should_offer(Some("/elsewhere/fernjo.AppImage"), home, false),
            "the AppImage moved: ask again"
        );
    }

    #[test]
    fn an_oversized_entry_is_offered_again_without_being_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        install(APP, home).unwrap();
        let (entry, _) = entry_paths(home);
        let mut text = desktop_entry(APP).unwrap().into_bytes();
        text.resize(usize::try_from(MAX_ENTRY_LEN).unwrap() + 1, b'\n');
        std::fs::write(&entry, text).unwrap();
        assert!(should_offer(Some(APP), home, false));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_entry_is_managed_by_the_user_and_never_offered() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        // an entry elsewhere (a dotfile manager's), linked at the entry's name: whatever it
        // says, and also when the link is dangling, the user manages it, so no dialog
        let real = home.join("elsewhere.desktop");
        std::fs::write(&real, "[Desktop Entry]\nName=other\n").unwrap();
        let (entry, _) = entry_paths(home);
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &entry).unwrap();
        assert!(!should_offer(Some(APP), home, false));
        std::fs::remove_file(&real).unwrap();
        assert!(!should_offer(Some(APP), home, false), "dangling link");
    }

    #[test]
    fn install_writes_the_icon_and_the_entry_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        install(APP, dir.path()).unwrap();
        let (entry, icon) = entry_paths(dir.path());
        assert_eq!(
            std::fs::read_to_string(&entry).unwrap(),
            desktop_entry(APP).unwrap()
        );
        assert_eq!(std::fs::read(&icon).unwrap(), ICON_BYTES);
        assert!(ICON_BYTES.starts_with(b"\x89PNG"));
        for d in [entry.parent().unwrap(), icon.parent().unwrap()] {
            let names: Vec<_> = std::fs::read_dir(d)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert!(names.iter().all(|n| !n.ends_with(".tmp")), "{names:?}");
        }
        // a second install replaces the entry
        install("/new/place.AppImage", dir.path()).unwrap();
        assert!(
            std::fs::read_to_string(&entry)
                .unwrap()
                .contains("/new/place.AppImage")
        );
    }

    #[test]
    fn the_appimage_path_must_be_absolute() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!should_offer(Some("fernjo.AppImage"), dir.path(), false));
        assert!(!should_offer(Some("./fernjo.AppImage"), dir.path(), false));
        assert!(should_offer(
            Some("/opt/fernjo.AppImage"),
            dir.path(),
            false
        ));
    }

    #[test]
    fn install_leaves_no_pid_named_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let (entry, icon) = entry_paths(dir.path());
        // a file at the old fixed `<name>.<pid>.tmp` name must neither block the install nor be
        // touched by it
        let mut planted = Vec::new();
        for target in [&entry, &icon] {
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            let mut name = target.file_name().unwrap().to_os_string();
            name.push(format!(".{}.tmp", std::process::id()));
            let path = target.with_file_name(name);
            std::fs::write(&path, b"planted").unwrap();
            planted.push(path);
        }
        install(APP, dir.path()).unwrap();
        for path in &planted {
            assert_eq!(std::fs::read(path).unwrap(), b"planted");
        }
        let planted_paths = planted;
        assert!(entry.exists() && icon.exists());
        // nothing else is left: the random temp names of the writer are renamed away
        for d in [entry.parent().unwrap(), icon.parent().unwrap()] {
            for e in std::fs::read_dir(d).unwrap() {
                let name = e.unwrap().path();
                if name.extension().is_some_and(|x| x == "tmp") {
                    assert!(planted_paths.contains(&name), "left over: {name:?}");
                }
            }
        }
    }

    #[test]
    fn install_refuses_an_unwritable_path_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert!(install("/a\nb", dir.path()).is_err());
        assert!(!entry_paths(dir.path()).0.exists());
        assert!(!entry_paths(dir.path()).1.exists());
    }

    #[test]
    fn the_dialog_says_when_the_icon_may_appear_and_has_no_gaps() {
        let text = offer_message(Path::new("/home/u/.local/share"), APP);
        assert_eq!(
            text,
            format!(
                "fernjo runs as an AppImage, so your desktop has no application menu entry for it \
                 and shows a placeholder icon in the title bar.\n\n\
                 Add fernjo to your application menu? This writes two small files below \
                 /home/u/.local/share and points the entry at\n{APP}\n\n\
                 The icon may appear in the start menu and dock only after you log out and in \
                 again.\n\n\
                 If you want to move the AppImage first, choose \"Not now\"; the question comes \
                 back at the next start."
            )
        );
        assert!(!text.contains("  "), "{text:?}");
    }

    #[test]
    fn the_dialog_names_the_folder_and_the_appimage_as_they_are() {
        let app = "/h/a b/100% \"{x}\".AppImage";
        let text = offer_message(Path::new("/h/a b/share"), app);
        assert!(text.contains(" below /h/a b/share and "), "{text:?}");
        assert!(text.contains(&format!("\n{app}\n")), "{text:?}");
    }

    #[test]
    fn an_entry_written_by_v0_4_0_is_not_asked_about_again() {
        let dir = tempfile::tempdir().unwrap();
        let (entry, _) = entry_paths(dir.path());
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        // a literal on purpose, not `desktop_entry`: this is the text v0.4.0 put on disk
        let written = format!(
            "[Desktop Entry]\nType=Application\nName=fernjo\nComment=SSH connection manager with a \
             connection tree and tabbed sessions.\nExec=\"{APP}\"\nIcon=fernjo\nStartupWMClass=fernjo\n\
             Categories=Development;\nTerminal=false\n"
        );
        std::fs::write(&entry, written).unwrap();
        assert!(!should_offer(Some(APP), dir.path(), false));
    }

    #[test]
    fn never_is_remembered_in_the_device_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_never(Some(dir.path())));
        assert!(!is_never(None));
        remember_never(dir.path()).unwrap();
        assert!(is_never(Some(dir.path())));
    }
}
