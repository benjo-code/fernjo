//! The macOS menu's Quit item (M14 review #105). The menu itself needs a Mac; what its item does
//! is tested on every OS with the mock runtime.

use std::sync::{Arc, Mutex};

use fernjo_app_lib::mac_menu::{self, QUIT_ID};
use tauri::menu::MenuEvent;
use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};
use tauri::{RunEvent, WebviewWindow, WebviewWindowBuilder, WindowEvent};

fn app_with_main_window() -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>) {
    let app = mock_builder()
        .build(mock_context(noop_assets()))
        .expect("mock app");
    let window = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("main window");
    (app, window)
}

/// Picks the menu item `id` and returns what the event loop then saw. The window is destroyed
/// afterwards, so the loop also ends when the item did nothing. Both happen inside the running
/// loop: before it runs, the mock removes a window without any event.
fn pick(id: &'static str) -> Vec<String> {
    let (app, window) = app_with_main_window();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    app.run(move |handle, event| match event {
        RunEvent::Ready => {
            mac_menu::on_event(handle, MenuEvent { id: id.into() });
            window.destroy().expect("destroy");
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { .. },
            ..
        } => sink
            .lock()
            .unwrap()
            .push(format!("close requested: {label}")),
        RunEvent::ExitRequested { code, .. } => sink
            .lock()
            .unwrap()
            .push(format!("exit requested: {code:?}")),
        _ => {}
    });
    seen.lock().unwrap().clone()
}

#[test]
fn the_quit_item_asks_the_main_window_to_close() {
    // `CloseRequested` is where fernjo asks about connected sessions and holds back while an
    // update installs; an exit code would pass both.
    assert_eq!(
        pick(QUIT_ID),
        ["close requested: main", "exit requested: None"]
    );
}

#[test]
fn another_menu_item_closes_nothing() {
    assert_eq!(pick("something-else"), ["exit requested: None"]);
}
