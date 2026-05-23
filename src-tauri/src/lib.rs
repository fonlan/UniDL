mod commands;
mod db;
mod engine_adapters;
mod models;
mod repositories;
mod services;

use std::{path::PathBuf, sync::Mutex};

use rusqlite::Connection;
use tauri::Manager;

pub struct AppState {
    connection: Mutex<Connection>,
    database_path: PathBuf,
}

impl AppState {
    fn new(connection: Connection, database_path: PathBuf) -> Self {
        Self {
            connection: Mutex::new(connection),
            database_path,
        }
    }

    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "database connection lock was poisoned".to_string())
    }

    fn database_path(&self) -> PathBuf {
        self.database_path.clone()
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
            let database_path = db::database_path(app.handle())?;
            let connection = db::connect_path(database_path.clone())?;
            app.manage(AppState::new(connection, database_path));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_download_tasks,
            commands::refresh_download_tasks,
            commands::create_download_task,
            commands::pause_download_tasks,
            commands::resume_download_tasks,
            commands::delete_download_tasks,
            commands::pause_all_unfinished_download_tasks,
            commands::resume_all_paused_download_tasks,
            commands::list_engine_settings,
            commands::save_engine_settings,
            commands::validate_engine_source_type
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UniDL");
}
