//! The macOS menu bar: Tauri's default menu with one difference, Quit (and with it Cmd+Q) is
//! fernjo's own item. The predefined one sends `terminate:`, which ends the process without
//! `RunEvent::ExitRequested`, so neither the close confirmation nor a running update install could
//! hold it back (M14 review #105). Quit from the Dock, a log out and a shutdown still use
//! `terminate:`, see `Docs/DECISIONS.md` "M14: Cmd+Q and the menu's Quit close the window".
//!
//! Only macOS installs the menu (`run`), but it compiles everywhere.

use tauri::menu::{
    AboutMetadata, HELP_SUBMENU_ID, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    WINDOW_SUBMENU_ID,
};
use tauri::{AppHandle, Manager, Runtime};

/// Id of fernjo's own Quit item.
pub const QUIT_ID: &str = "fernjo-quit";

/// Builds the menu bar. Without its Edit items, copy and paste do not work in a text field on
/// macOS.
///
/// # Errors
///
/// Fails when the system refuses to create a menu or an item.
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let info = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
        name: Some(info.name.clone()),
        version: Some(info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };
    let separator = || PredefinedMenuItem::separator(app);
    let quit = format!("Quit {}", info.name);

    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                info.name.clone(),
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about))?,
                    &separator()?,
                    &PredefinedMenuItem::services(app, None)?,
                    &separator()?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &separator()?,
                    &MenuItem::with_id(app, QUIT_ID, quit, true, Some("CmdOrCtrl+Q"))?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "File",
                true,
                &[&PredefinedMenuItem::close_window(app, None)?],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &separator()?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &Submenu::with_id_and_items(
                app,
                WINDOW_SUBMENU_ID,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                    &separator()?,
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?,
            &Submenu::with_id_and_items(app, HELP_SUBMENU_ID, "Help", true, &[])?,
        ],
    )
}

/// Quit closes the main window, so it runs exactly what the close button runs: the "sessions still
/// connected" dialog, the wait for a running install and the last sync push. The window is brought
/// to the front first, so a dialog is not asked from a minimised window.
pub fn on_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    if event.id() != QUIT_ID {
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        app.exit(0);
        return;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    let _ = window.close();
}
