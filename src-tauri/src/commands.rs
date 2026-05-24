use tauri::{AppHandle, Manager, State};

use crate::{
    engine_install,
    logger,
    models::{
        AppSettings, AppSettingsInput, CreateDownloadTaskInput, DownloadTask, EngineInstallResult,
        EngineKind, EngineSettings, EngineSettingsInput, SourceType,
    },
    services::{AppSettingsService, DownloadTaskService, EngineSettingsService},
    AppState,
};

#[tauri::command]
pub fn write_log(level: String, message: String) -> Result<(), String> {
    match level.as_str() {
        "info" => logger::info(message),
        "warn" => logger::warn(message),
        "error" => logger::error(message),
        _ => return Err(format!("unknown log level: {level}")),
    }
    Ok(())
}

#[tauri::command]
pub fn list_download_tasks(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .list_created_desc()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn take_pending_open_requests(
    state: State<'_, AppState>,
) -> Result<Vec<crate::system_open::OpenTaskRequest>, String> {
    state.take_pending_open_requests()
}

#[tauri::command]
pub fn get_torrent_files(source: String) -> Result<Vec<crate::torrent_metadata::TorrentFileEntry>, String> {
    crate::torrent_metadata::read_torrent_files(&source).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn refresh_download_tasks(state: State<'_, AppState>) -> Result<Vec<DownloadTask>, String> {
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .refresh_all()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn create_download_task(
    input: CreateDownloadTaskInput,
    state: State<'_, AppState>,
) -> Result<DownloadTask, String> {
    logger::info(format!(
        "creating download task: engine={:?}, source_type={:?}",
        input.engine, input.source_type
    ));
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .create(input)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn pause_download_tasks(ids: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    logger::info(format!("pausing download tasks: count={}", ids.len()));
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .pause_tasks(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resume_download_tasks(ids: Vec<String>, state: State<'_, AppState>) -> Result<(), String> {
    logger::info(format!("resuming download tasks: count={}", ids.len()));
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .resume_tasks(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_downloaded_file(id: String, state: State<'_, AppState>) -> Result<(), String> {
    logger::info(format!("opening downloaded file: task_id={id}"));
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .open_downloaded_file(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_download_tasks(
    ids: Vec<String>,
    delete_completed_files: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    logger::info(format!(
        "deleting download tasks: count={}, delete_completed_files={}",
        ids.len(), delete_completed_files
    ));
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .delete_tasks(&ids, delete_completed_files)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn pause_all_unfinished_download_tasks(state: State<'_, AppState>) -> Result<(), String> {
    logger::info("pausing all unfinished download tasks");
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .pause_all_unfinished()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn resume_all_paused_download_tasks(state: State<'_, AppState>) -> Result<(), String> {
    logger::info("resuming all paused download tasks");
    let connection = state.lock_connection()?;
    DownloadTaskService::new(&connection, state.database_path())
        .resume_all_paused()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let connection = state.lock_connection()?;
    AppSettingsService::new(&connection)
        .get()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn save_app_settings(
    settings: AppSettingsInput,
    app_handle: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppSettings, String> {
    logger::info(format!(
        "saving app settings: web_access_enabled={}",
        settings.web_access_enabled
    ));
    AppSettingsService::validate_input(&settings).map_err(|error| error.to_string())?;

    let connection = state.lock_connection()?;
    let next = AppSettingsService::new(&connection)
        .save(settings)
        .map_err(|error| error.to_string())?;
    drop(connection);

    state.apply_web_settings(app_handle, &next)?;

    Ok(next)
}

#[tauri::command]
pub fn get_system_download_dir(app_handle: AppHandle) -> Result<String, String> {
    app_handle
        .path()
        .download_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_managed_engine_executable_path(engine: EngineKind) -> Option<String> {
    engine_install::managed_executable_path(engine).map(|path| path.to_string_lossy().into_owned())
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
    logger::info(format!(
        "saving engine settings: id={}, engine={:?}",
        settings.id, settings.engine
    ));
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .save(settings)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_engine_settings(
    settings_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    logger::info(format!("deleting engine settings: id={settings_id}"));
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .delete(&settings_id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn install_latest_engine(
    settings_id: String,
    state: State<'_, AppState>,
) -> Result<EngineInstallResult, String> {
    logger::info(format!("installing latest engine: settings_id={settings_id}"));
    let connection = state.lock_connection()?;
    EngineSettingsService::new(&connection)
        .install_latest(&settings_id)
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
