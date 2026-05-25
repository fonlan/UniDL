mod commands;
mod db;
mod engine_adapters;
mod engine_install;
mod logger;
mod models;
mod repositories;
mod services;
mod system_open;
mod task_events;
mod torrent_metadata;
mod web_server;

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rusqlite::Connection;
use tauri::Manager;
#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;

pub struct AppState {
    connection: Mutex<Connection>,
    database_path: PathBuf,
    pending_open_sources: Arc<Mutex<Vec<system_open::OpenTaskRequest>>>,
    web_server: Mutex<Option<web_server::WebServerHandle>>,
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
            pending_open_sources: Arc::new(Mutex::new(system_open::source_requests(
                pending_open_sources,
            ))),
            web_server: Mutex::new(None),
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

    fn push_open_requests(
        &self,
        requests: Vec<system_open::OpenTaskRequest>,
    ) -> Result<(), String> {
        let mut pending = self
            .pending_open_sources
            .lock()
            .map_err(|_| "system open request lock was poisoned".to_string())?;
        pending.extend(requests);
        Ok(())
    }

    fn take_pending_open_requests(&self) -> Result<Vec<system_open::OpenTaskRequest>, String> {
        let mut pending = self
            .pending_open_sources
            .lock()
            .map_err(|_| "system open request lock was poisoned".to_string())?;
        Ok(std::mem::take(&mut *pending))
    }

    fn apply_web_settings(
        &self,
        app_handle: tauri::AppHandle,
        settings: &models::AppSettings,
    ) -> Result<(), String> {
        let mut web_server = self
            .web_server
            .lock()
            .map_err(|_| "web server lock was poisoned".to_string())?;

        if let Some(current) = web_server.take() {
            current.stop();
        }

        let next = web_server::start(
            app_handle,
            self.database_path(),
            Arc::clone(&self.pending_open_sources),
            settings,
        )
        .map_err(|error| error.to_string())?;
        *web_server = Some(next);

        Ok(())
    }
}

pub fn run() {
    logger::init().expect("failed to initialize UniDL logger");
    logger::info("UniDL starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let sources = system_open::parse_open_sources(argv);
            if !sources.is_empty() {
                logger::info(format!("received system open request: {} source(s)", sources.len()));
                let requests = system_open::source_requests(sources);
                app.state::<AppState>()
                    .push_open_requests(requests.clone())
                    .expect("failed to store system open request");
                system_open::emit_open_requests(app, requests)
                    .expect("failed to emit system open request");
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .setup(|app| {
            logger::info("Tauri setup started");
            #[cfg(any(windows, target_os = "linux"))]
            app.deep_link().register_all()?;
            system_open::register_torrent_file_association()?;

            let pending_open_sources = system_open::parse_open_sources(std::env::args());
            let database_path = db::database_path(app.handle())?;
            let connection = db::connect_path(database_path.clone())?;
            logger::info(format!("database connected: {}", database_path.display()));
            let app_settings = services::AppSettingsService::new(&connection).get()?;
            app.manage(AppState::new(
                connection,
                database_path.clone(),
                pending_open_sources,
            ));
            app.state::<AppState>()
                .apply_web_settings(app.handle().clone(), &app_settings)
                .map_err(std::io::Error::other)?;
            task_events::spawn_download_task_refresh_worker(
                app.handle().clone(),
                database_path.clone(),
            );
            logger::info("Tauri setup completed");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_download_tasks,
            commands::take_pending_open_requests,
            commands::refresh_download_tasks,
            commands::create_download_task,
            commands::get_torrent_files,
            commands::resolve_magnet_name,
            commands::pause_download_tasks,
            commands::resume_download_tasks,
            commands::open_downloaded_file,
            commands::delete_download_tasks,
            commands::pause_all_unfinished_download_tasks,
            commands::resume_all_paused_download_tasks,
            commands::get_app_settings,
            commands::save_app_settings,
            commands::get_system_download_dir,
            commands::get_managed_engine_executable_path,
            commands::list_engine_settings,
            commands::save_engine_settings,
            commands::delete_engine_settings,
            commands::update_engine_trackers,
            commands::install_latest_engine,
            commands::test_engine_connection,
            commands::validate_engine_source_type,
            commands::write_log
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UniDL");
}
