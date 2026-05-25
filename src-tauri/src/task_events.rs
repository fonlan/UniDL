use std::{path::PathBuf, thread, time::Duration};

use tauri::{AppHandle, Emitter, Runtime};

use crate::{db, services::DownloadTaskService};

pub const DOWNLOAD_TASKS_UPDATED_EVENT: &str = "download-tasks-updated";

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

        if let Err(error) =
            DownloadTaskService::new(&connection, database_path.clone()).refresh_all()
        {
            eprintln!("download task refresh worker refresh error: {error}");
            continue;
        }

        let _ = emit_download_tasks_updated(&app_handle);
    });
}
