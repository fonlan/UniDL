use tauri::State;

use crate::{models::DownloadTask, services::DownloadTaskService, AppState};

#[tauri::command]
pub fn list_download_tasks(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .list_created_desc()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn pause_download_tasks(ids: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .pause_tasks(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resume_download_tasks(ids: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .resume_tasks(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_download_tasks(ids: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .delete_tasks(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn pause_all_unfinished_download_tasks(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .pause_all_unfinished()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resume_all_paused_download_tasks(state: State<'_, AppState>) -> Result<(), String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .resume_all_paused()
        .map_err(|error| error.to_string())
}
