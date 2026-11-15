mod commands;
mod data_folder;
mod state;
mod vault_commands;

use tauri::Manager;

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
        // while the builder is assembled, i.e. also in a second launch; it normally only reads.)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::open_for_app(fernjo_core::locate()))
        .invoke_handler(tauri::generate_handler![
            commands::handlers::list_tree,
            commands::handlers::get_node,
            commands::handlers::create_folder,
            commands::handlers::create_connection,
            commands::handlers::update_node,
            commands::handlers::move_node,
            commands::handlers::duplicate_node,
            commands::handlers::delete_node,
            vault_commands::handlers::vault_status,
            vault_commands::handlers::vault_create,
            vault_commands::handlers::vault_unlock,
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
            data_folder::handlers::apply_data_folder,
            data_folder::handlers::choose_data_folder,
            data_folder::handlers::restart_app,
        ])
        .run(tauri::generate_context!())
        .expect("error while running fernjo");
}
