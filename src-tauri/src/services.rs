use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use rusqlite::Connection;
use uuid::Uuid;

use crate::{
    engine_adapters, engine_install,
    models::{
        engine_supports_source_type, supported_source_types, AppSettings, AppSettingsInput,
        CreateDownloadTaskInput, DownloadStatus, DownloadTask, EngineInstallResult, EngineKind,
        EngineSettings, EngineSettingsInput, NewDownloadTask, SourceType,
    },
    repositories::{AppSettingsRepository, DownloadTaskRepository, EngineSettingsRepository},
};

pub struct DownloadTaskService<'connection> {
    connection: &'connection Connection,
    database_path: PathBuf,
    repository: DownloadTaskRepository<'connection>,
}

impl<'connection> DownloadTaskService<'connection> {
    pub fn new(connection: &'connection Connection, database_path: PathBuf) -> Self {
        Self {
            connection,
            database_path,
            repository: DownloadTaskRepository::new(connection),
        }
    }

    pub fn list_created_desc(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        self.repository.list_created_desc()
    }

    pub fn create(&self, input: CreateDownloadTaskInput) -> Result<DownloadTask, Box<dyn Error>> {
        validate_create_task_input(&input)?;

        let engine_settings = self.resolve_engine_settings(&input)?;
        if !engine_settings.enabled {
            return Err(format!("{} is disabled", engine_settings.id).into());
        }
        if !engine_settings
            .supported_source_types
            .contains(&input.source_type)
        {
            return Err(format!(
                "{} does not support {} tasks",
                engine_settings.id,
                input.source_type.as_db()
            )
            .into());
        }

        let id = Uuid::new_v4().to_string();
        let task_input = NewDownloadTask {
            source_type: input.source_type,
            source: input.source,
            engine_settings_id: engine_settings.id.clone(),
            engine: engine_settings.engine,
            file_name: input.file_name,
            save_path: input.save_path,
            engine_args: input.engine_args,
            selected_file_indexes: input.selected_file_indexes,
            browser_cookies: input.browser_cookies,
        };
        self.repository.create(&id, &task_input)?;
        let task = self.repository.get_by_id(&id)?;

        match engine_adapters::add_task(&engine_settings, &task, self.database_path.clone()) {
            Ok(state) => {
                self.apply_engine_state(&task.id, state)?;
                self.repository.get_by_id(&task.id)
            }
            Err(error) => {
                self.repository.delete_tasks(&[task.id])?;
                Err(error)
            }
        }
    }

    pub fn torrent_files(
        &self,
        id: &str,
    ) -> Result<Vec<crate::torrent_metadata::TorrentFileEntry>, Box<dyn Error>> {
        let task = self.repository.get_by_id(id)?;
        if !matches!(task.source_type, SourceType::Magnet | SourceType::Torrent) {
            return Err(format!("{} does not have torrent files", task.source_type.as_db()).into());
        }
        let engine_settings =
            EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
        engine_adapters::task_torrent_files(&engine_settings, &task)
    }

    pub fn update_file_selection(
        &self,
        id: &str,
        selected_file_indexes: Vec<i64>,
    ) -> Result<DownloadTask, Box<dyn Error>> {
        if selected_file_indexes.is_empty() {
            return Err("selected file indexes cannot be empty".into());
        }
        if selected_file_indexes.iter().any(|index| *index < 1) {
            return Err("selected file indexes must start from 1".into());
        }

        let task = self.repository.get_by_id(id)?;
        if !matches!(task.source_type, SourceType::Magnet | SourceType::Torrent) {
            return Err(format!(
                "{} does not support file selection",
                task.source_type.as_db()
            )
            .into());
        }
        let engine_settings =
            EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
        engine_adapters::update_task_file_selection(
            &engine_settings,
            &task,
            &selected_file_indexes,
        )?;
        self.repository
            .update_selected_file_indexes(id, Some(&selected_file_indexes))?;
        self.repository.get_by_id(id)
    }

    pub fn refresh_all(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        let tasks = self.repository.list_created_desc()?;
        for task in tasks {
            if matches!(
                task.status,
                DownloadStatus::Completed | DownloadStatus::Deleted | DownloadStatus::Failed
            ) || task.engine_task_id.is_none()
            {
                continue;
            }

            if let Err(error) = self.refresh_one(&task) {
                self.repository.mark_failed(&task.id, &error.to_string())?;
            }
        }

        self.repository.list_created_desc()
    }

    pub fn pause_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            match engine_adapters::pause_task(&engine_settings, &task) {
                Ok(state) => self.apply_engine_state(&task.id, state)?,
                Err(error) => {
                    self.repository.mark_failed(&task.id, &error.to_string())?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn resume_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            match engine_adapters::resume_task(&engine_settings, &task, self.database_path.clone())
            {
                Ok(state) => self.apply_engine_state(&task.id, state)?,
                Err(error) => {
                    self.repository.mark_failed(&task.id, &error.to_string())?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn open_downloaded_file(&self, id: &str) -> Result<(), Box<dyn Error>> {
        let task = self.repository.get_by_id(id)?;
        if task.engine == EngineKind::Aria2 || task.engine == EngineKind::YtDlp {
            let path = downloaded_entry_path(&task);
            fs::metadata(&path)?;
            return open_path(&path);
        }

        Err("download task does not use a local download file managed by UniDL".into())
    }

    pub fn delete_tasks(
        &self,
        ids: &[String],
        delete_completed_files: bool,
    ) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            let delete_downloaded = delete_completed_files
                && (task.engine != EngineKind::QBittorrent || downloaded_entry_path(&task).exists());
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            let delete_result =
                if task.engine_task_id.is_none() && task.status == DownloadStatus::Failed {
                    Ok(())
                } else {
                    engine_adapters::delete_task(&engine_settings, &task, delete_downloaded)
                };
            match delete_result {
                Ok(()) => {
                    if delete_downloaded && task.engine != EngineKind::QBittorrent {
                        if let Err(error) = delete_downloaded_entry(&task) {
                            self.repository.delete_tasks(&[task.id])?;
                            return Err(error);
                        }
                    }
                    self.repository.delete_tasks(&[task.id])?;
                }
                Err(error) => {
                    if task.status == DownloadStatus::Completed
                        && task.engine != EngineKind::QBittorrent
                    {
                        if delete_downloaded {
                            if let Err(remove_error) = delete_downloaded_entry(&task) {
                                self.repository.delete_tasks(&[task.id])?;
                                return Err(remove_error);
                            }
                        }
                        self.repository.delete_tasks(&[task.id])?;
                        continue;
                    }
                    self.repository.mark_failed(&task.id, &error.to_string())?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn pause_all_unfinished(&self) -> Result<(), Box<dyn Error>> {
        let ids = self
            .repository
            .list_unfinished()?
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        self.pause_tasks(&ids)
    }

    pub fn resume_all_paused(&self) -> Result<(), Box<dyn Error>> {
        let ids = self
            .repository
            .list_paused()?
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        self.resume_tasks(&ids)
    }

    fn refresh_one(&self, task: &DownloadTask) -> Result<(), Box<dyn Error>> {
        let engine_settings =
            EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
        let state = engine_adapters::refresh_task(&engine_settings, task)?;
        self.apply_engine_state(&task.id, state)
    }

    fn apply_engine_state(
        &self,
        task_id: &str,
        state: engine_adapters::EngineTaskState,
    ) -> Result<(), Box<dyn Error>> {
        self.repository.update_engine_state(
            task_id,
            state.status,
            state.progress,
            state.speed_bytes_per_sec,
            state.downloaded_bytes,
            state.total_bytes,
            state.engine_task_id.as_deref(),
            state.error_message.as_deref(),
        )?;
        if let Some(file_name) = state
            .file_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let task = self.repository.get_by_id(task_id)?;
            if task.source_type == SourceType::Magnet && task.file_name != file_name {
                self.repository.update_file_name(task_id, file_name)?;
            }
        }
        if state.status == DownloadStatus::Completed {
            let task = self.repository.get_by_id(task_id)?;
            let app_settings = AppSettingsRepository::new(self.connection).get()?;
            if matches_private_download_domain(&task.source, &app_settings.private_download_domains)
            {
                self.repository.delete_tasks(&[task.id])?;
            }
        }
        Ok(())
    }

    fn resolve_engine_settings(
        &self,
        input: &CreateDownloadTaskInput,
    ) -> Result<EngineSettings, Box<dyn Error>> {
        if let Some(engine_settings_id) = input.engine_settings_id.as_deref() {
            let settings =
                EngineSettingsRepository::new(self.connection).get(engine_settings_id)?;
            if settings.engine != input.engine {
                return Err(format!(
                    "engine settings {} does not match {}",
                    settings.id,
                    input.engine.as_db()
                )
                .into());
            }
            return Ok(settings);
        }

        let settings = EngineSettingsRepository::new(self.connection).list_all()?;
        settings
            .into_iter()
            .find(|settings| {
                settings.engine == input.engine
                    && settings.enabled
                    && settings.supported_source_types.contains(&input.source_type)
            })
            .ok_or_else(|| {
                format!(
                    "no enabled {} settings supports {} tasks",
                    input.engine.as_db(),
                    input.source_type.as_db()
                )
                .into()
            })
    }
}

fn delete_downloaded_entry(task: &DownloadTask) -> Result<(), Box<dyn Error>> {
    let path = downloaded_entry_path(task);
    if task.status != DownloadStatus::Completed {
        remove_downloaded_entry(&downloaded_partial_entry_path(&path))?;
    }
    remove_downloaded_entry(&path)
}

fn remove_downloaded_entry(path: &Path) -> Result<(), Box<dyn Error>> {
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::InvalidInput
                    | io::ErrorKind::InvalidFilename
            ) =>
        {
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::IsADirectory => {
            match fs::remove_dir_all(&path) {
                Ok(()) => Ok(()),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound
                            | io::ErrorKind::InvalidInput
                            | io::ErrorKind::InvalidFilename
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(error.into()),
            }
        }
        Err(error) => Err(error.into()),
    }
}

fn downloaded_entry_path(task: &DownloadTask) -> PathBuf {
    let save_path = Path::new(&task.save_path);
    let path = save_path.join(&task.file_name);
    if path.exists() || task.engine != EngineKind::YtDlp || task.status != DownloadStatus::Completed
    {
        return path;
    }

    resolve_ytdlp_downloaded_entry_path(save_path, &task.file_name).unwrap_or(path)
}

fn resolve_ytdlp_downloaded_entry_path(save_path: &Path, file_name: &str) -> Option<PathBuf> {
    let file_name = engine_adapters::sanitize_ytdlp_output_name(file_name.trim());
    if file_name.is_empty() {
        return None;
    }

    let sanitized_path = save_path.join(&file_name);
    if sanitized_path.exists() {
        return Some(sanitized_path);
    }

    if Path::new(&file_name).extension().is_some() {
        return None;
    }

    let prefix = format!("{file_name}.");
    let entries = fs::read_dir(save_path).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && !name.ends_with(".part") {
            return Some(path);
        }
    }

    None
}

fn downloaded_partial_entry_path(path: &Path) -> PathBuf {
    let mut partial_path = path.as_os_str().to_os_string();
    partial_path.push(".part");
    PathBuf::from(partial_path)
}

fn open_path(path: &Path) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    command.spawn()?;
    Ok(())
}

fn validate_create_task_input(input: &CreateDownloadTaskInput) -> Result<(), Box<dyn Error>> {
    if input.source.trim().is_empty() {
        return Err("download source is required".into());
    }
    if input.file_name.trim().is_empty() {
        return Err("file name is required".into());
    }
    if input.save_path.trim().is_empty() {
        return Err("download path is required".into());
    }
    if let Some(indexes) = &input.selected_file_indexes {
        if indexes.is_empty() {
            return Err("selected file indexes cannot be empty".into());
        }
        if indexes.iter().any(|index| *index < 1) {
            return Err("selected file indexes must start from 1".into());
        }
    }
    Ok(())
}

pub struct EngineSettingsService<'connection> {
    repository: EngineSettingsRepository<'connection>,
}

pub struct AppSettingsService<'connection> {
    repository: AppSettingsRepository<'connection>,
}

impl<'connection> AppSettingsService<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self {
            repository: AppSettingsRepository::new(connection),
        }
    }

    pub fn get(&self) -> Result<AppSettings, Box<dyn Error>> {
        self.repository.get()
    }

    pub fn save(&self, input: AppSettingsInput) -> Result<AppSettings, Box<dyn Error>> {
        Self::validate_input(&input)?;
        self.repository.save(&input)
    }

    pub fn validate_input(input: &AppSettingsInput) -> Result<(), Box<dyn Error>> {
        if input.web_access_enabled && input.web_access_password.trim().is_empty() {
            return Err("web access password is required".into());
        }
        crate::web_server::bind_address_from_url(&input.web_access_url)
            .map_err(|error| -> Box<dyn Error> { error.to_string().into() })?;
        for domain in &input.private_download_domains {
            if normalize_domain(domain).is_empty() {
                return Err("private download domain cannot be empty".into());
            }
        }
        Ok(())
    }
}

fn source_hostname(source: &str) -> Option<String> {
    let (_, rest) = source.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?.split('@').next_back()?;
    let host = host.split(':').next()?.trim().trim_matches(['[', ']']);
    if host.is_empty() {
        return None;
    }

    Some(host.to_lowercase())
}

fn normalize_domain(domain: &str) -> String {
    domain
        .trim()
        .to_lowercase()
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .to_string()
}

fn matches_private_download_domain(source: &str, domains: &[String]) -> bool {
    let Some(hostname) = source_hostname(source) else {
        return false;
    };

    domains.iter().any(|domain| {
        let domain = normalize_domain(domain);
        !domain.is_empty() && (hostname == domain || hostname.ends_with(&format!(".{domain}")))
    })
}

impl<'connection> EngineSettingsService<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self {
            repository: EngineSettingsRepository::new(connection),
        }
    }

    pub fn list_all(&self) -> Result<Vec<EngineSettings>, Box<dyn Error>> {
        self.repository.list_all()
    }

    pub fn get(&self, settings_id: &str) -> Result<EngineSettings, Box<dyn Error>> {
        self.repository.get(settings_id)
    }

    pub fn save(&self, input: EngineSettingsInput) -> Result<EngineSettings, Box<dyn Error>> {
        let input = normalize_engine_settings_input(input)?;
        self.repository.save(&input)
    }

    pub fn delete(&self, settings_id: &str) -> Result<(), Box<dyn Error>> {
        let current = self.repository.get(settings_id)?;
        if self.repository.has_download_tasks(settings_id)? {
            return Err(format!("{} is used by download tasks", current.name).into());
        }

        self.repository.delete(settings_id)
    }

    pub fn install_latest(&self, settings_id: &str) -> Result<EngineInstallResult, Box<dyn Error>> {
        let current = self.repository.get(settings_id)?;
        let installed = engine_install::install_latest(current.engine)?;
        let next = self.save(EngineSettingsInput {
            id: current.id,
            engine: current.engine,
            name: current.name,
            enabled: current.enabled,
            executable_path: Some(installed.executable_path.to_string_lossy().into_owned()),
            default_download_dir: current.default_download_dir,
            default_args: current.default_args,
            connection_url: current.connection_url,
            username: current.username,
            password: current.password,
            remote_path: current.remote_path,
            supported_source_types: current.supported_source_types,
            preferred_domains: current.preferred_domains,
            tracker_subscription_url: current.tracker_subscription_url,
            trackers: current.trackers,
            priority: current.priority,
        })?;

        Ok(EngineInstallResult {
            settings: next,
            version: installed.version,
        })
    }

    pub fn test_connection(&self, input: EngineSettingsInput) -> Result<(), Box<dyn Error>> {
        let input = normalize_engine_settings_input(input)?;
        let settings = EngineSettings {
            id: input.id,
            engine: input.engine,
            name: input.name,
            enabled: input.enabled,
            executable_path: input.executable_path,
            default_download_dir: input.default_download_dir,
            default_args: input.default_args,
            connection_url: input.connection_url,
            username: input.username,
            password: input.password,
            remote_path: input.remote_path,
            supported_source_types: input.supported_source_types,
            preferred_domains: input.preferred_domains,
            tracker_subscription_url: input.tracker_subscription_url,
            trackers: input.trackers,
            priority: input.priority,
            updated_at: String::new(),
        };
        crate::engine_adapters::test_connection(&settings)
    }

    pub fn update_tracker_subscription(
        &self,
        settings_id: &str,
        subscription_url: &str,
    ) -> Result<EngineSettings, Box<dyn Error>> {
        let current = self.repository.get(settings_id)?;
        if current.engine != EngineKind::Aria2 {
            return Err("tracker subscription is only supported for aria2".into());
        }

        let trackers = fetch_trackers(subscription_url)?;
        if trackers.is_empty() {
            return Err("tracker subscription returned no trackers".into());
        }

        self.save(EngineSettingsInput {
            id: current.id,
            engine: current.engine,
            name: current.name,
            enabled: current.enabled,
            executable_path: current.executable_path,
            default_download_dir: current.default_download_dir,
            default_args: current.default_args,
            connection_url: current.connection_url,
            username: current.username,
            password: current.password,
            remote_path: current.remote_path,
            supported_source_types: current.supported_source_types,
            preferred_domains: current.preferred_domains,
            tracker_subscription_url: Some(subscription_url.trim().to_string()),
            trackers,
            priority: current.priority,
        })
    }

    pub fn validate_source_type(
        &self,
        engine: EngineKind,
        source_type: SourceType,
    ) -> Result<(), Box<dyn Error>> {
        if engine_supports_source_type(engine, source_type) {
            Ok(())
        } else {
            Err(format!(
                "{} does not support {} tasks",
                engine.as_db(),
                source_type.as_db()
            )
            .into())
        }
    }
}

fn normalize_engine_settings_input(
    input: EngineSettingsInput,
) -> Result<EngineSettingsInput, Box<dyn Error>> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("engine settings name is required".into());
    }

    if input.engine == EngineKind::QBittorrent
        && input.remote_path.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err("qBittorrent remote save path is required".into());
    }

    let supported = supported_source_types(input.engine);
    for source_type in &input.supported_source_types {
        if !supported.contains(source_type) {
            return Err(format!(
                "{} does not support {} tasks",
                input.engine.as_db(),
                source_type.as_db()
            )
            .into());
        }
    }
    let supported_source_types = supported
        .into_iter()
        .filter(|source_type| input.supported_source_types.contains(source_type))
        .collect();

    Ok(EngineSettingsInput {
        name,
        tracker_subscription_url: input.tracker_subscription_url.and_then(|value| {
            let trimmed = value.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        }),
        trackers: normalize_trackers(input.trackers),
        supported_source_types,
        ..input
    })
}

fn fetch_trackers(subscription_url: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let url = subscription_url.trim();
    if url.is_empty() {
        return Err("tracker subscription url is required".into());
    }

    let response = reqwest::blocking::get(url)?;
    if !response.status().is_success() {
        return Err(format!("tracker subscription failed: {}", response.status()).into());
    }

    Ok(normalize_trackers(parse_trackers(&response.text()?)))
}

fn parse_trackers(value: &str) -> Vec<String> {
    value
        .split(|character: char| character.is_whitespace() || character == ',' || character == ';')
        .map(ToOwned::to_owned)
        .collect()
}

fn normalize_trackers(trackers: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for tracker in trackers {
        let tracker = tracker.trim().to_string();
        if tracker.is_empty() || !tracker.contains("://") {
            continue;
        }
        if !normalized
            .iter()
            .any(|item: &String| item.eq_ignore_ascii_case(&tracker))
        {
            normalized.push(tracker);
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        path::PathBuf,
        sync::{Arc, Mutex},
        thread,
    };

    use uuid::Uuid;

    use super::*;
    use crate::db;

    #[test]
    fn migrated_database_has_no_default_engine_settings() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");

        let settings = EngineSettingsService::new(&connection)
            .list_all()
            .expect("engine settings should list");
        assert!(settings.is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn engine_settings_can_be_renamed_and_deleted_when_unused() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let service = EngineSettingsService::new(&connection);

        let saved = service
            .save(EngineSettingsInput {
                id: "aria2-primary".to_string(),
                engine: EngineKind::Aria2,
                name: "aria2".to_string(),
                enabled: false,
                executable_path: Some("C:\\Tools\\aria2c.exe".to_string()),
                default_download_dir: String::new(),
                default_args: "--continue=true --max-connection-per-server=16 --split=16 --min-split-size=1M --file-allocation=none".to_string(),
                connection_url: Some("http://127.0.0.1:6800/jsonrpc".to_string()),
                username: None,
                password: None,
                remote_path: None,
                supported_source_types: vec![SourceType::Http, SourceType::Ftp],
                preferred_domains: Vec::new(),
                priority: 0,
            })
            .expect("engine settings should save");
        assert_eq!(saved.name, "aria2");

        let renamed = service
            .save(EngineSettingsInput {
                id: saved.id.clone(),
                engine: saved.engine,
                name: "fast aria2".to_string(),
                enabled: saved.enabled,
                executable_path: saved.executable_path,
                default_download_dir: saved.default_download_dir,
                default_args: saved.default_args,
                connection_url: saved.connection_url,
                username: saved.username,
                password: saved.password,
                remote_path: saved.remote_path,
                supported_source_types: saved.supported_source_types,
                preferred_domains: Vec::new(),
                priority: saved.priority,
            })
            .expect("engine settings should rename");
        assert_eq!(renamed.name, "fast aria2");

        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id,
                    source_type,
                    source,
                    engine_settings_id,
                    engine,
                    file_name,
                    status,
                    save_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
                (
                    "task-using-aria2",
                    SourceType::Http.as_db(),
                    "http://example.test/file.bin",
                    renamed.id.as_str(),
                    EngineKind::Aria2.as_db(),
                    "file.bin",
                    DownloadStatus::Queued.as_db(),
                    "C:\\Downloads",
                ),
            )
            .expect("download task should insert");
        assert!(service.delete(&renamed.id).is_err());

        connection
            .execute(
                "UPDATE download_tasks SET status = ?1 WHERE id = ?2",
                (DownloadStatus::Deleted.as_db(), "task-using-aria2"),
            )
            .expect("download task should mark deleted");
        service
            .delete(&renamed.id)
            .expect("unused engine settings should delete");
        assert!(service
            .list_all()
            .expect("engine settings should list")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_unfinished_tasks_removes_downloaded_file_and_folder() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("file.bin");
        let download_folder = download_dir.join("album");
        fs::write(&download_file, b"partial").expect("download file should write");
        fs::create_dir_all(&download_folder).expect("download folder should create");
        fs::write(download_folder.join("track.bin"), b"partial")
            .expect("download folder file should write");

        insert_aria2_task(
            &connection,
            "unfinished-file-task",
            DownloadStatus::Running,
            download_dir.to_string_lossy().as_ref(),
            "file.bin",
        );
        insert_aria2_task(
            &connection,
            "unfinished-folder-task",
            DownloadStatus::Paused,
            download_dir.to_string_lossy().as_ref(),
            "album",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(
                &[
                    "unfinished-file-task".to_string(),
                    "unfinished-folder-task".to_string(),
                ],
                false,
            )
            .expect("unfinished tasks should delete downloaded entries");

        assert!(!download_file.exists());
        assert!(!download_folder.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn completed_private_domain_task_is_removed_from_list() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        AppSettingsService::new(&connection)
            .save(AppSettingsInput {
                web_access_enabled: false,
                web_access_password: String::new(),
                web_access_url: "http://127.0.0.1:18080".to_string(),
                private_download_domains: vec!["example.test".to_string()],
            })
            .expect("app settings should save");
        insert_aria2_task(
            &connection,
            "private-domain-task",
            DownloadStatus::Running,
            "C:\\Downloads",
            "file.bin",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .apply_engine_state(
                "private-domain-task",
                engine_adapters::EngineTaskState {
                    status: DownloadStatus::Completed,
                    progress: 100.0,
                    speed_bytes_per_sec: 0,
                    downloaded_bytes: 1,
                    total_bytes: 1,
                    engine_task_id: None,
                    error_message: None,
                },
            )
            .expect("completed private-domain task should update");

        assert!(service
            .list_created_desc()
            .expect("tasks should list")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_completed_task_keeps_downloaded_file_when_not_requested() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("file.bin");
        fs::write(&download_file, b"complete").expect("download file should write");

        insert_aria2_task(
            &connection,
            "completed-keep-task",
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "file.bin",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["completed-keep-task".to_string()], false)
            .expect("completed task should delete without removing file");

        assert!(download_file.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_completed_task_removes_downloaded_folder_when_requested() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let download_dir = temp_download_dir();
        let download_folder = download_dir.join("album");
        fs::create_dir_all(&download_folder).expect("download folder should create");
        fs::write(download_folder.join("track.bin"), b"complete")
            .expect("download file should write");

        insert_aria2_task(
            &connection,
            "completed-remove-task",
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "album",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["completed-remove-task".to_string()], true)
            .expect("completed task should delete folder when requested");

        assert!(!download_folder.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_completed_aria2_task_deletes_local_file_after_rpc_bad_request() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, server) = start_fake_aria2_bad_request();
        save_aria2_settings_with_url(&connection, url);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("file.bin");
        fs::write(&download_file, b"complete").expect("download file should write");

        insert_aria2_task(
            &connection,
            "completed-aria2-bad-request-task",
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "file.bin",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["completed-aria2-bad-request-task".to_string()], true)
            .expect("completed task should delete locally after aria2 rejects history cleanup");

        server.join().expect("fake aria2 should finish");
        assert!(!download_file.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_completed_aria2_task_removes_record_when_local_file_delete_fails() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, server) = start_fake_aria2_status("complete", 2);
        save_aria2_settings_with_url(&connection, url);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let blocked_parent = download_dir.join("blocked-parent");
        fs::write(&blocked_parent, b"not a directory").expect("blocked parent should write");

        insert_aria2_task(
            &connection,
            "completed-aria2-local-delete-failed-task",
            DownloadStatus::Completed,
            blocked_parent.to_string_lossy().as_ref(),
            "file.bin",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(
                &["completed-aria2-local-delete-failed-task".to_string()],
                true,
            )
            .expect_err("local file delete failure should still be reported");

        server.join().expect("fake aria2 should finish");
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_completed_task_ignores_invalid_download_path() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let download_dir = temp_download_dir();

        insert_aria2_task(
            &connection,
            "completed-invalid-path-task",
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "missing<file>.bin",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["completed-invalid-path-task".to_string()], true)
            .expect("completed task should delete even with invalid missing path");

        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_completed_qbittorrent_task_ignores_missing_local_file() {
        let (base_url, requests, server) = start_fake_qbittorrent_with_bodies(2);
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");

        EngineSettingsService::new(&connection)
            .save(EngineSettingsInput {
                id: "qbittorrent".to_string(),
                engine: EngineKind::QBittorrent,
                name: "qBittorrent".to_string(),
                enabled: true,
                executable_path: None,
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: Some(base_url),
                username: Some("admin".to_string()),
                password: Some("adminadmin".to_string()),
                remote_path: Some(String::new()),
                supported_source_types: vec![SourceType::Magnet, SourceType::Torrent],
                preferred_domains: Vec::new(),
                priority: 0,
            })
            .expect("qBittorrent settings should save");

        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id,
                    source_type,
                    source,
                    engine_settings_id,
                    engine,
                    engine_task_id,
                    file_name,
                    status,
                    save_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                (
                    "completed-qbittorrent-missing-file",
                    SourceType::Magnet.as_db(),
                    "magnet:?xt=urn:btih:1234567890abcdef1234",
                    "qbittorrent",
                    EngineKind::QBittorrent.as_db(),
                    "abcdef123456",
                    "missing.mkv",
                    DownloadStatus::Completed.as_db(),
                    temp_download_dir().to_string_lossy().as_ref(),
                ),
            )
            .expect("completed qBittorrent task should insert");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["completed-qbittorrent-missing-file".to_string()], true)
            .expect("completed qBittorrent task should delete without local file");

        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        server.join().expect("fake qBittorrent should finish");
        let requests = requests.lock().expect("requests should lock").clone();
        assert!(requests
            .last()
            .expect("delete request should exist")
            .contains("deleteFiles=false"));

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_failed_ytdlp_task_without_downloaded_entry_removes_local_record() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_ytdlp_settings(&connection);

        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id,
                    source_type,
                    source,
                    engine_settings_id,
                    engine,
                    engine_task_id,
                    file_name,
                    status,
                    save_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                (
                    "failed-ytdlp-missing-file",
                    SourceType::Http.as_db(),
                    "http://example.test/missing.mp4",
                    "yt-dlp",
                    EngineKind::YtDlp.as_db(),
                    "1234",
                    "missing.mp4",
                    DownloadStatus::Paused.as_db(),
                    temp_download_dir().to_string_lossy().as_ref(),
                ),
            )
            .expect("unfinished yt-dlp task should insert");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["failed-ytdlp-missing-file".to_string()], false)
            .expect("failed yt-dlp task with no downloaded entry should delete locally");

        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_unfinished_ytdlp_task_removes_partial_entry() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_ytdlp_settings(&connection);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let partial_file = download_dir.join("video.mp4.part");
        fs::write(&partial_file, b"partial").expect("partial file should write");

        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id,
                    source_type,
                    source,
                    engine_settings_id,
                    engine,
                    engine_task_id,
                    file_name,
                    status,
                    save_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                (
                    "paused-ytdlp-part-file",
                    SourceType::Http.as_db(),
                    "http://example.test/video.mp4",
                    "yt-dlp",
                    EngineKind::YtDlp.as_db(),
                    "1234",
                    "video.mp4",
                    DownloadStatus::Paused.as_db(),
                    download_dir.to_string_lossy().as_ref(),
                ),
            )
            .expect("unfinished yt-dlp task should insert");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["paused-ytdlp-part-file".to_string()], true)
            .expect("unfinished yt-dlp task should delete partial file");

        assert!(!partial_file.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_unfinished_aria2_task_keeps_local_file_when_requested() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, server) = start_fake_aria2_status("active", 2);
        save_aria2_settings_with_url(&connection, url);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let partial_file = download_dir.join("movie.mkv");
        fs::write(&partial_file, b"partial").expect("partial file should write");

        insert_aria2_task(
            &connection,
            "running-aria2-keep-file-task",
            DownloadStatus::Running,
            download_dir.to_string_lossy().as_ref(),
            "movie.mkv",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["running-aria2-keep-file-task".to_string()], false)
            .expect("unfinished aria2 task should keep local file when requested");

        server.join().expect("fake aria2 should finish");
        assert!(partial_file.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn completed_ytdlp_path_uses_real_ext_file() {
        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("Page Title.mp4");
        fs::write(&download_file, b"complete").expect("download file should write");

        let mut task = local_file_task(
            EngineKind::YtDlp,
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "Page Title",
        );

        assert_eq!(downloaded_entry_path(&task), download_file);

        task.file_name = "Page Title.mp4".to_string();
        assert_eq!(
            downloaded_entry_path(&task),
            download_dir.join("Page Title.mp4")
        );

        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn completed_ytdlp_path_uses_sanitized_file_name() {
        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("探索_海洋.mp4");
        fs::write(&download_file, b"complete").expect("download file should write");

        let task = local_file_task(
            EngineKind::YtDlp,
            DownloadStatus::Completed,
            download_dir.to_string_lossy().as_ref(),
            "探索:海洋",
        );

        assert_eq!(downloaded_entry_path(&task), download_file);

        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn failed_task_creation_does_not_leave_local_task() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let service = DownloadTaskService::new(&connection, database_path.clone());
        let result = service.create(CreateDownloadTaskInput {
            source_type: SourceType::Http,
            source: "http://example.test/file.bin".to_string(),
            engine: EngineKind::Aria2,
            engine_settings_id: Some("aria2".to_string()),
            file_name: "file.bin".to_string(),
            save_path: "C:\\Downloads".to_string(),
            engine_args: String::new(),
            selected_file_indexes: None,
            browser_cookies: None,
        });

        assert!(result.is_err());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn deleting_failed_task_without_engine_task_id_removes_local_record() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);

        let repository = DownloadTaskRepository::new(&connection);
        repository
            .create(
                "failed-without-engine-id",
                &NewDownloadTask {
                    source_type: SourceType::Http,
                    source: "http://example.test/file.bin".to_string(),
                    engine_settings_id: "aria2".to_string(),
                    engine: EngineKind::Aria2,
                    file_name: "file.bin".to_string(),
                    save_path: "C:\\Downloads".to_string(),
                    engine_args: String::new(),
                    selected_file_indexes: None,
                    browser_cookies: None,
                },
            )
            .expect("failed task should insert");
        repository
            .mark_failed("failed-without-engine-id", "engine failed before id")
            .expect("failed task should mark failed");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["failed-without-engine-id".to_string()], false)
            .expect("failed task without engine id should delete locally");

        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn qbittorrent_task_lifecycle_add_pause_resume_delete() {
        let (base_url, hits, server) = start_fake_qbittorrent(8);
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");

        EngineSettingsService::new(&connection)
            .save(EngineSettingsInput {
                id: "qbittorrent".to_string(),
                engine: EngineKind::QBittorrent,
                name: "qBittorrent".to_string(),
                enabled: true,
                executable_path: None,
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: Some(base_url),
                username: Some("admin".to_string()),
                password: Some("adminadmin".to_string()),
                remote_path: Some(String::new()),
                supported_source_types: vec![SourceType::Magnet, SourceType::Torrent],
                preferred_domains: Vec::new(),
                priority: 0,
            })
            .expect("qBittorrent settings should save");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        let task = service
            .create(CreateDownloadTaskInput {
                source_type: SourceType::Magnet,
                source: "magnet:?xt=urn:btih:ABCDEF123456&dn=unidl".to_string(),
                engine: EngineKind::QBittorrent,
                engine_settings_id: Some("qbittorrent".to_string()),
                file_name: "unidl".to_string(),
                save_path: "C:\\Downloads".to_string(),
                engine_args: String::new(),
                selected_file_indexes: None,
                browser_cookies: None,
            })
            .expect("task should be added through qBittorrent");
        assert_eq!(task.status, DownloadStatus::Running);
        assert_eq!(task.engine_task_id.as_deref(), Some("abcdef123456"));

        service
            .pause_tasks(std::slice::from_ref(&task.id))
            .expect("task should pause");
        let paused = service.list_created_desc().expect("task should list");
        assert_eq!(paused[0].status, DownloadStatus::Paused);

        service
            .resume_tasks(std::slice::from_ref(&task.id))
            .expect("task should resume");
        let resumed = service.list_created_desc().expect("task should list");
        assert_eq!(resumed[0].status, DownloadStatus::Running);

        service
            .delete_tasks(std::slice::from_ref(&task.id), true)
            .expect("task should delete");
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        server.join().expect("fake qBittorrent should finish");
        let paths = hits.lock().expect("hits should lock").clone();
        assert_eq!(
            paths,
            vec![
                "/api/v2/auth/login",
                "/api/v2/torrents/add",
                "/api/v2/auth/login",
                "/api/v2/torrents/pause",
                "/api/v2/auth/login",
                "/api/v2/torrents/resume",
                "/api/v2/auth/login",
                "/api/v2/torrents/delete",
            ]
        );

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    fn save_unreachable_aria2_settings(connection: &Connection) {
        save_aria2_settings_with_url(connection, "http://127.0.0.1:1/jsonrpc".to_string());
    }

    fn save_aria2_settings_with_url(connection: &Connection, connection_url: String) {
        EngineSettingsService::new(connection)
            .save(EngineSettingsInput {
                id: "aria2".to_string(),
                engine: EngineKind::Aria2,
                name: "aria2".to_string(),
                enabled: true,
                executable_path: None,
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: Some(connection_url),
                username: None,
                password: None,
                remote_path: None,
                supported_source_types: vec![
                    SourceType::Http,
                    SourceType::Ftp,
                    SourceType::Magnet,
                    SourceType::Torrent,
                ],
                preferred_domains: Vec::new(),
                priority: 0,
            })
            .expect("aria2 settings should save");
    }

    fn start_fake_aria2_bad_request() -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake aria2 should bind");
        let address = listener
            .local_addr()
            .expect("fake aria2 should have address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fake aria2 request should arrive");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).expect("request should read");
            stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .expect("response should write");
        });

        (format!("http://{address}/jsonrpc"), server)
    }

    fn start_fake_aria2_status(
        status: &'static str,
        expected_requests: usize,
    ) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake aria2 should bind");
        let address = listener
            .local_addr()
            .expect("fake aria2 should have address");
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(expected_requests) {
                let mut stream = stream.expect("fake aria2 stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .expect("request should include body");
                let request: serde_json::Value =
                    serde_json::from_str(body).expect("request should be json");
                let method = request
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    .expect("request should include method");
                let response = if method == "aria2.tellStatus" {
                    serde_json::json!({"jsonrpc": "2.0", "id": "unidl", "result": {"status": status}})
                } else {
                    serde_json::json!({"jsonrpc": "2.0", "id": "unidl", "result": "OK"})
                };
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
            }
        });

        (format!("http://{address}/jsonrpc"), server)
    }

    fn save_ytdlp_settings(connection: &Connection) {
        EngineSettingsService::new(connection)
            .save(EngineSettingsInput {
                id: "yt-dlp".to_string(),
                engine: EngineKind::YtDlp,
                name: "yt-dlp".to_string(),
                enabled: true,
                executable_path: Some("yt-dlp.exe".to_string()),
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: None,
                username: None,
                password: None,
                remote_path: None,
                supported_source_types: vec![SourceType::Http, SourceType::Ftp],
                preferred_domains: Vec::new(),
                priority: 0,
            })
            .expect("yt-dlp settings should save");
    }

    fn insert_aria2_task(
        connection: &Connection,
        id: &str,
        status: DownloadStatus,
        save_path: &str,
        file_name: &str,
    ) {
        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
                    id,
                    source_type,
                    source,
                    engine_settings_id,
                    engine,
                    engine_task_id,
                    file_name,
                    status,
                    save_path
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                (
                    id,
                    SourceType::Http.as_db(),
                    "http://example.test/file.bin",
                    "aria2",
                    EngineKind::Aria2.as_db(),
                    "gid",
                    file_name,
                    status.as_db(),
                    save_path,
                ),
            )
            .expect("download task should insert");
    }

    fn local_file_task(
        engine: EngineKind,
        status: DownloadStatus,
        save_path: &str,
        file_name: &str,
    ) -> DownloadTask {
        DownloadTask {
            id: "local-file-task".to_string(),
            source_type: SourceType::Http,
            source: "http://example.test/file".to_string(),
            engine_settings_id: engine.as_db().to_string(),
            engine,
            engine_task_id: Some("engine-task".to_string()),
            file_name: file_name.to_string(),
            status,
            progress: 100.0,
            speed_bytes_per_sec: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            save_path: save_path.to_string(),
            engine_args: String::new(),
            selected_file_indexes: None,
            browser_cookies: None,
            created_at: String::new(),
            completed_at: Some(String::new()),
            error_message: None,
        }
    }

    fn temp_download_dir() -> PathBuf {
        std::env::temp_dir().join(format!("unidl-download-test-{}", Uuid::new_v4()))
    }

    fn start_fake_qbittorrent(
        expected_requests: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake server should bind");
        let address = listener
            .local_addr()
            .expect("fake server should have address");
        let hits = Arc::new(Mutex::new(Vec::new()));
        let server_hits = Arc::clone(&hits);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(expected_requests) {
                let mut stream = stream.expect("fake server stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("request should include path")
                    .to_string();
                server_hits.lock().expect("hits should lock").push(path);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .expect("response should write");
            }
        });

        (format!("http://{address}"), hits, server)
    }

    fn start_fake_qbittorrent_with_bodies(
        expected_requests: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake server should bind");
        let address = listener
            .local_addr()
            .expect("fake server should have address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(expected_requests) {
                let mut stream = stream.expect("fake server stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .expect("request should include body")
                    .to_string();
                server_requests
                    .lock()
                    .expect("requests should lock")
                    .push(body);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .expect("response should write");
            }
        });

        (format!("http://{address}"), requests, server)
    }

    fn temp_database_path() -> PathBuf {
        std::env::temp_dir().join(format!("unidl-test-{}.sqlite3", Uuid::new_v4()))
    }
}
