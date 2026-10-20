pub mod commands;
pub mod core;
pub mod db;
pub mod error;
pub mod models;
pub mod state;

use tauri::Manager;

use crate::state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::root::inspect_root,
            commands::root::initialize_root,
            commands::root::open_root,
            commands::root::get_root_snapshot,
            commands::root::refresh_merchants,
            commands::root::close_root,
            commands::root::create_merchant,
            commands::batch::start_batch,
            commands::batch::cancel_job,
            commands::batch::get_active_job,
            commands::batch::list_batches,
            commands::batch::get_batch_detail,
            commands::batch::regenerate_report,
            commands::system::open_result_dir,
            commands::system::open_report,
            commands::system::open_library,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
