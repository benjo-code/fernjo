mod app_menu;
mod app_settings;
mod clipboard;
mod commands;
mod connect_plan;
mod data_folder;
mod idle_lock;
mod import_commands;
mod nav_guard;
mod open_tabs;
mod rdp_commands;
mod session_commands;
mod session_flow;
mod sessions;
mod state;
mod sync_commands;
mod sync_port;
mod sync_runner;
pub mod updater;
mod vault_commands;

use std::sync::atomic::Ordering;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

pub use state::AppState;

/// Starts the fernjo desktop application.
///
/// # Panics
///
/// Panics if the Tauri runtime fails to start.
pub fn run() {
    tauri::Builder::default()
        // The single-instance plugin must stay the first plugin so a second launch is
        // intercepted before any other plugin initialises. (`AppState::open_for_app` below runs
        // while the builder is assembled, i.e. also in a second launch; it reads the data files and
        // the keychain (auto-unlock; it never deletes the item) and, on macOS, may trigger a keychain
        // prompt.)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(nav_guard::plugin())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            // terminal links open through `open_external_url` in Rust; the plugin's JS
            // click handler (opens any <a href> in the webview) is not wanted
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .manage(AppState::open_for_app(fernjo_core::locate()))
        // Linux AppImage: the compositor takes the title bar icon from an installed `.desktop`
        // entry, which an AppImage has none of; `app_menu` offers to write one (never without the
        // user's answer). `Window::set_icon` cannot do it under Wayland.
        .setup(|app| {
            #[cfg(target_os = "linux")]
            app_menu::offer(app.handle());
            // Background sync (a no-op while sync is off). `AppState` is built, and with it the
            // startup auto-unlock tried, before `setup` runs.
            sync_runner::spawn_triggers(app.handle());
            updater::spawn(app.handle().clone());
            idle_lock::spawn(app.handle().clone());
            Ok(())
        })
        // A reload or a crashed page leaves its sessions running without a UI (the page's
        // `closeAll` on `pagehide` is fire-and-forget), so every page load ends them. The first
        // load of the app finds none.
        .on_page_load(|webview, payload| {
            if webview.label() == "main"
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Started)
            {
                let state = webview.state::<AppState>();
                state.sessions.close_all();
                // a picked or inspected import must not outlive the page that asked for it
                state.clear_pending();
            }
        })
        // Closing the window ends every session, so ask first while one is connected. The dialog
        // is non-blocking (the handler runs on the main thread); its callback runs on another
        // thread and ends the window with `destroy`, which does not raise `CloseRequested` again.
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if matches!(event, tauri::WindowEvent::Focused(true)) {
                window.state::<AppState>().sync_wake.focus();
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                // the update's files are being replaced: closing now could cut the write
                if updater::install_blocks_exit(updater::installing(&state.updater), None) {
                    api.prevent_close();
                    return;
                }
                let connected = state.sessions.connected_count();
                if !sessions::needs_close_confirmation(connected) {
                    // No session to ask about: with sync set up the window waits (at most a
                    // few seconds, on another thread) for a last push, then closes.
                    if sync_runner::hold_close(&state) {
                        api.prevent_close();
                        sync_runner::destroy_after_push(window.clone());
                    }
                    return;
                }
                api.prevent_close();
                ask_to_close(window, connected, sessions::CloseThen::Exit);
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::handlers::list_tree,
            commands::handlers::reload_tree,
            commands::handlers::get_node,
            commands::handlers::create_folder,
            commands::handlers::create_connection,
            commands::handlers::update_node,
            commands::handlers::move_node,
            commands::handlers::move_nodes,
            commands::handlers::duplicate_node,
            commands::handlers::delete_node,
            vault_commands::handlers::vault_status,
            vault_commands::handlers::vault_create,
            vault_commands::handlers::vault_unlock,
            vault_commands::handlers::vault_unlock_older,
            vault_commands::handlers::vault_lock,
            vault_commands::handlers::vault_change_password,
            vault_commands::handlers::vault_set_auto_unlock,
            vault_commands::handlers::list_credentials,
            vault_commands::handlers::get_credential,
            vault_commands::handlers::create_credential,
            vault_commands::handlers::update_credential,
            vault_commands::handlers::credential_usage,
            vault_commands::handlers::delete_credential,
            vault_commands::handlers::reveal_credential,
            data_folder::handlers::get_data_folder,
            data_folder::handlers::choose_data_folder,
            data_folder::handlers::restart_app,
            app_settings::handlers::get_theme,
            app_settings::handlers::set_theme,
            app_settings::handlers::get_vault_auto_lock,
            app_settings::handlers::set_vault_auto_lock,
            app_settings::handlers::note_activity,
            session_commands::handlers::get_terminal_settings,
            session_commands::handlers::set_terminal_settings,
            session_commands::handlers::open_session,
            session_commands::handlers::restore_session,
            open_tabs::handlers::get_open_tabs,
            open_tabs::handlers::set_reopen_tabs,
            open_tabs::handlers::save_open_tabs,
            session_commands::handlers::quick_parse,
            clipboard::handlers::clipboard_read_text,
            session_commands::handlers::open_quick_session,
            session_commands::handlers::session_input,
            session_commands::handlers::session_resize,
            session_commands::handlers::session_ack,
            session_commands::handlers::close_session,
            session_commands::handlers::reconnect_session,
            session_commands::handlers::answer_prompt,
            session_commands::handlers::resume_session,
            session_commands::handlers::open_external_url,
            rdp_commands::handlers::rdp_input,
            rdp_commands::handlers::rdp_clipboard,
            import_commands::handlers::import_pick,
            import_commands::handlers::import_inspect,
            import_commands::handlers::import_apply,
            import_commands::handlers::import_cancel,
            import_commands::handlers::export_connections,
            import_commands::handlers::export_backup,
            sync_commands::handlers::sync_status,
            sync_commands::handlers::sync_setup,
            sync_commands::handlers::sync_pairing_code,
            sync_commands::handlers::sync_join,
            sync_commands::handlers::sync_now,
            sync_commands::handlers::sync_reseed,
            sync_commands::handlers::sync_edit_secret,
            sync_commands::handlers::sync_leave,
            sync_commands::handlers::sync_dismiss_findings,
            updater::handlers::update_status,
            updater::handlers::update_check,
            updater::handlers::update_install,
            updater::handlers::update_skip,
            updater::handlers::set_check_for_updates,
            updater::handlers::update_open_release_page,
        ])
        .build(tauri::generate_context!())
        .expect("error while building fernjo")
        .run(|app, event| {
            // Cmd+Q on macOS (and any other quit that is not a window close) asks like the
            // window close does. An explicit exit code is never intercepted.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                let state = app.state::<AppState>();
                // an exit without a code waits for an install in progress; the restart that
                // follows it carries a code and passes
                if updater::install_blocks_exit(updater::installing(&state.updater), code) {
                    api.prevent_exit();
                    return;
                }
                let connected = state.sessions.connected_count();
                let prompt_open = state.close_prompt_open.load(Ordering::SeqCst);
                match sessions::exit_action(code, connected, prompt_open) {
                    sessions::ExitAction::Allow => {}
                    sessions::ExitAction::Prevent => api.prevent_exit(),
                    sessions::ExitAction::Ask => {
                        // with no main window there is nothing to ask in: the exit goes ahead
                        if let Some(window) = app.get_webview_window("main") {
                            api.prevent_exit();
                            ask_to_close(
                                &window.as_ref().window(),
                                connected,
                                sessions::CloseThen::Exit,
                            );
                        }
                    }
                }
            }
        });
}

/// Asks whether to end `connected` sessions by closing (or, for `restart_app`, restarting)
/// fernjo; one dialog at a time (`close_prompt_open`, shared by the window close, Cmd+Q and the
/// restart). The dialog is non-blocking; on "Close" or "Restart" its callback ends the sessions
/// and, after a last sync push, destroys the window (which raises no `CloseRequested`; with no
/// session left the following `ExitRequested` is allowed) or requests the restart (whose
/// `ExitRequested` carries an exit code and is always allowed).
pub(crate) fn ask_to_close(window: &tauri::Window, connected: usize, then: sessions::CloseThen) {
    let state = window.state::<AppState>();
    if state.close_prompt_open.swap(true, Ordering::SeqCst) {
        return;
    }
    let prompt = sessions::close_prompt(connected, then);
    let window = window.clone();
    window
        .dialog()
        .message(prompt.message)
        .title(prompt.title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            prompt.confirm.into(),
            "Keep open".into(),
        ))
        .show({
            let window = window.clone();
            move |confirmed| {
                window
                    .state::<AppState>()
                    .close_prompt_open
                    .store(false, Ordering::SeqCst);
                if confirmed {
                    window.state::<AppState>().sessions.close_all();
                    sync_runner::finish_after_push(window.clone(), then);
                }
            }
        });
}
