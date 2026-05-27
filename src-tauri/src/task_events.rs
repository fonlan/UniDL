use std::{path::PathBuf, thread, time::Duration};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{db, services::DownloadTaskService, AppState};

pub const DOWNLOAD_TASKS_UPDATED_EVENT: &str = "download-tasks-updated";
const DOWNLOAD_TASK_AUTO_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

pub fn emit_download_tasks_updated<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    app.emit(DOWNLOAD_TASKS_UPDATED_EVENT, ())
}

pub fn spawn_download_task_refresh_worker(app_handle: AppHandle, database_path: PathBuf) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(2));

        let connection = match db::connect_path(database_path.clone()) {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("download task refresh worker connection error: {error}");
                continue;
            }
        };

        let tasks = match DownloadTaskService::new(&connection, database_path.clone()).refresh_all()
        {
            Ok(tasks) => tasks,
            Err(error) => {
                eprintln!("download task refresh worker refresh error: {error}");
                continue;
            }
        };

        if let Err(error) = app_handle
            .state::<AppState>()
            .handle_refreshed_tasks(&app_handle, &tasks)
        {
            eprintln!("download task refresh worker side effect error: {error}");
        }

        let _ = emit_download_tasks_updated(&app_handle);
    });
}

pub fn spawn_download_task_auto_cleanup_worker(app_handle: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(DOWNLOAD_TASK_AUTO_CLEANUP_INTERVAL);

        if let Err(error) = app_handle
            .state::<AppState>()
            .apply_auto_download_task_cleanup(&app_handle)
        {
            eprintln!("download task auto cleanup worker error: {error}");
        }
    });
}
