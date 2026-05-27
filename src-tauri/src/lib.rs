mod commands;
mod db;
mod engine_adapters;
mod engine_install;
mod logger;
mod models;
mod repositories;
mod services;
mod system_open;
mod system_sleep;
mod task_events;
mod torrent_metadata;
mod web_server;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use rusqlite::Connection;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt;
#[cfg(any(windows, target_os = "linux"))]
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_notification::NotificationExt;

pub struct AppState {
    connection: Mutex<Connection>,
    database_path: PathBuf,
    app_settings: Mutex<models::AppSettings>,
    pending_open_sources: Arc<Mutex<Vec<system_open::OpenTaskRequest>>>,
    magnet_file_cache: Arc<Mutex<HashMap<String, Vec<torrent_metadata::TorrentFileEntry>>>>,
    web_server: Mutex<Option<web_server::WebServerHandle>>,
    sleep_state: Mutex<system_sleep::SleepState>,
    notified_completed_task_ids: Mutex<HashSet<String>>,
}

impl AppState {
    fn new(
        connection: Connection,
        database_path: PathBuf,
        app_settings: models::AppSettings,
        pending_open_sources: Vec<String>,
    ) -> Result<Self, String> {
        let notified_completed_task_ids = repositories::DownloadTaskRepository::new(&connection)
            .list_created_desc()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|task| task.status == models::DownloadStatus::Completed)
            .map(|task| task.id)
            .collect();

        Ok(Self {
            connection: Mutex::new(connection),
            database_path,
            app_settings: Mutex::new(app_settings),
            pending_open_sources: Arc::new(Mutex::new(system_open::source_requests(
                pending_open_sources,
            ))),
            magnet_file_cache: Arc::new(Mutex::new(HashMap::new())),
            web_server: Mutex::new(None),
            sleep_state: Mutex::new(system_sleep::SleepState::new()),
            notified_completed_task_ids: Mutex::new(notified_completed_task_ids),
        })
    }

    fn lock_connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        self.connection
            .lock()
            .map_err(|_| "database connection lock was poisoned".to_string())
    }

    fn database_path(&self) -> PathBuf {
        self.database_path.clone()
    }

    fn app_settings(&self) -> Result<models::AppSettings, String> {
        self.app_settings
            .lock()
            .map(|settings| settings.clone())
            .map_err(|_| "app settings lock was poisoned".to_string())
    }

    fn set_app_settings(&self, settings: models::AppSettings) -> Result<(), String> {
        let mut current = self
            .app_settings
            .lock()
            .map_err(|_| "app settings lock was poisoned".to_string())?;
        *current = settings;
        Ok(())
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

    fn cache_magnet_files(
        &self,
        key: String,
        files: Vec<torrent_metadata::TorrentFileEntry>,
    ) -> Result<(), String> {
        let mut cache = self
            .magnet_file_cache
            .lock()
            .map_err(|_| "magnet file cache lock was poisoned".to_string())?;
        cache.insert(key, files);
        Ok(())
    }

    fn cached_magnet_files(
        &self,
        key: &str,
    ) -> Result<Option<Vec<torrent_metadata::TorrentFileEntry>>, String> {
        let cache = self
            .magnet_file_cache
            .lock()
            .map_err(|_| "magnet file cache lock was poisoned".to_string())?;
        Ok(cache.get(key).cloned())
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

    fn apply_web_settings_if_changed(
        &self,
        app_handle: tauri::AppHandle,
        current: &models::AppSettings,
        next: &models::AppSettings,
    ) -> Result<(), String> {
        if web_settings_changed(current, next) {
            self.apply_web_settings(app_handle, next)?;
        }
        Ok(())
    }

    fn refresh_sleep_prevention(&self) -> Result<(), String> {
        let settings = self.app_settings()?;
        let has_active_downloads = {
            let connection = self.lock_connection()?;
            repositories::DownloadTaskRepository::new(&connection)
                .has_active_downloads()
                .map_err(|error| error.to_string())?
        };
        let prevent_sleep = (settings.prevent_sleep_when_downloading_enabled
            && has_active_downloads)
            || (settings.prevent_sleep_when_web_access_enabled && settings.web_access_enabled);
        self.sleep_state
            .lock()
            .map_err(|_| "sleep state lock was poisoned".to_string())?
            .set_prevent_sleep(prevent_sleep)
    }

    fn handle_refreshed_tasks(
        &self,
        app_handle: &tauri::AppHandle,
        tasks: &[models::DownloadTask],
    ) -> Result<(), String> {
        self.refresh_sleep_prevention()?;
        let settings = self.app_settings()?;

        let mut notified = self
            .notified_completed_task_ids
            .lock()
            .map_err(|_| "completed notification lock was poisoned".to_string())?;
        for task in tasks
            .iter()
            .filter(|task| task.status == models::DownloadStatus::Completed)
        {
            if !notified.insert(task.id.clone()) {
                continue;
            }
            if !settings.download_completion_notification_enabled {
                continue;
            }
            if let Err(error) = app_handle
                .notification()
                .builder()
                .title("下载完成")
                .body(format!("{} 已下载完成", task.file_name))
                .show()
            {
                logger::error(format!(
                    "failed to show completion notification: task_id={}, error={error}",
                    task.id
                ));
            }
        }
        Ok(())
    }
}

fn web_settings_changed(current: &models::AppSettings, next: &models::AppSettings) -> bool {
    current.web_access_enabled != next.web_access_enabled
        || current.web_access_password != next.web_access_password
        || current.web_access_url != next.web_access_url
}

fn show_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show-main-window", "显示主窗口", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "退出 UniDL", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
    let icon = app
        .default_window_icon()
        .expect("default window icon is required for tray")
        .clone();

    TrayIconBuilder::with_id("main")
        .tooltip("UniDL")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app_handle, event| match event.id.as_ref() {
            "show-main-window" => show_main_window(app_handle),
            "quit" => app_handle.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn setup_close_to_tray(app: &tauri::App) {
    let app_handle = app.handle().clone();
    if let Some(window) = app.get_webview_window("main") {
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let settings = match app_handle.state::<AppState>().app_settings() {
                    Ok(settings) => settings,
                    Err(error) => {
                        logger::error(format!("failed to read app settings on close: {error}"));
                        return;
                    }
                };

                if settings.close_to_tray_enabled {
                    api.prevent_close();
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
            }
        });
    }
}

fn apply_autostart_settings(
    app_handle: &tauri::AppHandle,
    settings: &models::AppSettings,
) -> Result<(), String> {
    let autostart = app_handle.autolaunch();
    let is_enabled = autostart.is_enabled().map_err(|error| error.to_string())?;

    if settings.auto_start_enabled && !is_enabled {
        autostart.enable().map_err(|error| error.to_string())?;
    }
    if !settings.auto_start_enabled && is_enabled {
        autostart.disable().map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn launched_by_autostart() -> bool {
    std::env::args().any(|arg| arg == "--autostart")
}

fn hide_main_window_if_needed(app: &tauri::App, settings: &models::AppSettings) {
    if settings.auto_start_minimized_to_tray && launched_by_autostart() {
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.hide();
        }
    }
}

pub fn run() {
    logger::init().expect("failed to initialize UniDL logger");
    logger::info("UniDL starting");

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let sources = system_open::parse_open_sources(argv);
            if !sources.is_empty() {
                logger::info(format!(
                    "received system open request: {} source(s)",
                    sources.len()
                ));
                let requests = system_open::source_requests(sources);
                app.state::<AppState>()
                    .push_open_requests(requests.clone())
                    .expect("failed to store system open request");
                system_open::emit_open_requests(app, requests)
                    .expect("failed to emit system open request");
            }

            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
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
            app.manage(
                AppState::new(
                    connection,
                    database_path.clone(),
                    app_settings.clone(),
                    pending_open_sources,
                )
                .map_err(std::io::Error::other)?,
            );
            setup_tray(app)?;
            setup_close_to_tray(app);
            apply_autostart_settings(app.handle(), &app_settings).map_err(std::io::Error::other)?;
            app.state::<AppState>()
                .apply_web_settings(app.handle().clone(), &app_settings)
                .map_err(std::io::Error::other)?;
            app.state::<AppState>()
                .refresh_sleep_prevention()
                .map_err(std::io::Error::other)?;
            hide_main_window_if_needed(app, &app_settings);
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
            commands::check_download_file_conflict,
            commands::get_torrent_files,
            commands::get_task_torrent_files,
            commands::update_task_file_selection,
            commands::list_remote_directories,
            commands::resolve_magnet_name,
            commands::pause_download_tasks,
            commands::resume_download_tasks,
            commands::open_downloaded_file,
            commands::open_download_directory,
            commands::delete_download_tasks,
            commands::clear_download_records,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn app_settings() -> models::AppSettings {
        models::AppSettings {
            web_access_enabled: false,
            web_access_password: String::new(),
            web_access_url: "http://127.0.0.1:18080".to_string(),
            private_download_domains: Vec::new(),
            app_proxy_url: String::new(),
            auto_start_enabled: false,
            auto_start_minimized_to_tray: false,
            close_to_tray_enabled: false,
            download_completion_notification_enabled: false,
            prevent_sleep_when_downloading_enabled: false,
            prevent_sleep_when_web_access_enabled: false,
        }
    }

    #[test]
    fn notification_and_sleep_settings_do_not_restart_web_server() {
        let current = app_settings();
        let mut next = current.clone();
        next.download_completion_notification_enabled = true;
        next.prevent_sleep_when_downloading_enabled = true;

        assert!(!web_settings_changed(&current, &next));
    }

    #[test]
    fn web_access_settings_restart_web_server() {
        let current = app_settings();
        let mut next = current.clone();
        next.web_access_enabled = true;

        assert!(web_settings_changed(&current, &next));

        let mut next = current.clone();
        next.web_access_password = "secret".to_string();

        assert!(web_settings_changed(&current, &next));

        let mut next = current.clone();
        next.web_access_url = "http://127.0.0.1:18081".to_string();

        assert!(web_settings_changed(&current, &next));
    }
}
