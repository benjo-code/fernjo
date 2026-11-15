mod commands;
mod state;

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running fernjo");
}
