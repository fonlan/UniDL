use tauri::State;

use crate::{
    models::{
        CreateDownloadTaskInput, DownloadTask, EngineKind, EngineSettings,
        EngineSettingsInput, SourceType,
    },
    services::{DownloadTaskService, EngineSettingsService},
    AppState,
};

#[tauri::command]
pub fn list_download_tasks(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .list_created_desc()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_download_task(
    input: CreateDownloadTaskInput,
    state: State<'_, AppState>,
) -> Result<DownloadTask, String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection)
        .create(input)
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

#[tauri::command]
pub fn list_engine_settings(state: State<'_, AppState>) -> Result<Vec<EngineSettings>, String> {
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .list_all()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_engine_settings(
    settings: EngineSettingsInput,
    state: State<'_, AppState>,
) -> Result<EngineSettings, String> {
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .save(settings)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn validate_engine_source_type(
    engine: EngineKind,
    source_type: SourceType,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .validate_source_type(engine, source_type)
        .map_err(|error| error.to_string())
}
