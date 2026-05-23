mod commands;
mod db;
mod engine_adapters;
mod models;
mod repositories;
mod services;
mod system_open;

use std::{path::PathBuf, sync::Mutex};

use rusqlite::Connection;
use tauri::Manager;
#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

pub struct AppState {
    connection: Mutex<Connection>,
    database_path: PathBuf,
    pending_open_sources: Mutex<Vec<String>>,
}

impl AppState {
    fn new(
        connection: Connection,
        database_path: PathBuf,
        pending_open_sources: Vec<String>,
    ) -> Self {
        Self {
            connection: Mutex::new(connection),
            database_path,
            pending_open_sources: Mutex::new(pending_open_sources),
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

    fn push_open_sources(&self, sources: Vec<String>) -> Result<(), String> {
        let mut pending = self
            .pending_open_sources
            .lock()
            .map_err(|_| "system open request lock was poisoned".to_string())?;
        pending.extend(sources);
        Ok(())
    }

    fn take_pending_open_sources(&self) -> Result<Vec<String>, String> {
        let mut pending = self
            .pending_open_sources
            .lock()
            .map_err(|_| "system open request lock was poisoned".to_string())?;
        Ok(std::mem::take(&mut *pending))
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let sources = system_open::parse_open_sources(argv);
            if !sources.is_empty() {
                app.state::<AppState>()
                    .push_open_sources(sources.clone())
                    .expect("failed to store system open request");
                system_open::emit_open_sources(app, sources)
                    .expect("failed to emit system open request");
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            #[cfg(any(windows, target_os = "linux"))]
            app.deep_link().register_all()?;
            system_open::register_torrent_file_association()?;

            let pending_open_sources = system_open::parse_open_sources(std::env::args());
            let database_path = db::database_path(app.handle())?;
            let connection = db::connect_path(database_path.clone())?;
            app.manage(AppState::new(
                connection,
                database_path,
                pending_open_sources,
            ));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_download_tasks,
            commands::take_pending_open_requests,
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
