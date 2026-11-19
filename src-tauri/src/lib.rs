mod app_settings;
mod commands;
mod connect_plan;
mod data_folder;
mod import_commands;
mod nav_guard;
mod session_commands;
mod session_flow;
mod sessions;
mod state;
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::open_for_app(fernjo_core::locate()))
        // Candidate fix for the placeholder title bar icon on Linux (Wayland): apply the default
        // window icon (the one tauri-codegen already embeds) explicitly once the window exists, to
        // test whether setting it at that point changes the title bar. A no-op if the toolkit
        // already used it. A failure is ignored, never fatal.
        .setup(|app| {
            #[cfg(target_os = "linux")]
            if let (Some(window), Some(icon)) =
                (app.get_webview_window("main"), app.default_window_icon())
            {
                let _ = window.set_icon(icon.clone());
            }
            #[cfg(not(target_os = "linux"))]
            let _ = app;
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
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                let connected = state.sessions.connected_count();
                if !sessions::needs_close_confirmation(connected) {
                    return;
                }
                api.prevent_close();
                if state.close_prompt_open.swap(true, Ordering::SeqCst) {
                    return;
                }
                let noun = if connected == 1 {
                    "session is"
                } else {
                    "sessions are"
                };
                let window = window.clone();
                window
                    .dialog()
                    .message(format!(
                        "{connected} {noun} still connected. Closing fernjo ends them."
                    ))
                    .title("Close fernjo?")
                    .kind(MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::OkCancelCustom(
                        "Close".into(),
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
                                let _ = window.destroy();
                            }
                        }
                    });
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
            session_commands::handlers::get_terminal_settings,
            session_commands::handlers::set_terminal_settings,
            session_commands::handlers::open_session,
            session_commands::handlers::session_input,
            session_commands::handlers::session_resize,
            session_commands::handlers::session_ack,
            session_commands::handlers::close_session,
            session_commands::handlers::reconnect_session,
            session_commands::handlers::answer_prompt,
            session_commands::handlers::resume_session,
            session_commands::handlers::open_external_url,
            import_commands::handlers::import_pick,
            import_commands::handlers::import_inspect,
            import_commands::handlers::import_apply,
            import_commands::handlers::import_cancel,
            import_commands::handlers::export_connections,
            import_commands::handlers::export_backup,
        ])
        .run(tauri::generate_context!())
        .expect("error while running fernjo");
}
