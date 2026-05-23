mod commands;
mod db;
mod models;
mod repositories;
mod services;

use std::sync::Mutex;

use rusqlite::Connection;
use tauri::Manager;

pub struct AppState {
    connection: Mutex<Connection>,
}

impl AppState {
    fn new(connection: Connection) -> Self {
        Self {
            connection: Mutex::new(connection),
        }
    }

    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "database connection lock was poisoned".to_string())
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            let connection = db::connect(app.handle())?;
            app.manage(AppState::new(connection));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_download_tasks,
            commands::pause_download_tasks,
            commands::resume_download_tasks,
            commands::delete_download_tasks,
            commands::pause_all_unfinished_download_tasks,
            commands::resume_all_paused_download_tasks
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UniDL");
}
