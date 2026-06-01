use std::{
    collections::HashMap,
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use reqwest::{blocking::Client, header::CONTENT_LENGTH};
use rusqlite::Connection;
use uuid::Uuid;

use crate::{
    engine_adapters, engine_install, logger,
    models::{
        engine_supports_source_type, supported_source_types, AppSettings, AppSettingsInput,
        CreateDownloadTaskInput, DownloadDuplicateCheck, DownloadDuplicateKind,
        DownloadDuplicateMatch, DownloadDuplicateTaskState, DownloadFileConflict, DownloadStatus,
        DownloadTask, EngineInstallResult, EngineKind, EngineSettings, EngineSettingsInput,
        FileConflictAction, NewDownloadTask, SourceType,
    },
    repositories::{AppSettingsRepository, DownloadTaskRepository, EngineSettingsRepository},
};

const APP_HTTP_USER_AGENT: &str = "UniDL";
const APP_HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const DUPLICATE_HTTP_METADATA_TIMEOUT: Duration = Duration::from_secs(8);

pub fn build_app_http_client(proxy_url: Option<&str>) -> Result<Client, Box<dyn Error>> {
    build_app_http_client_with_timeout(proxy_url, APP_HTTP_TIMEOUT)
}

fn build_app_http_client_with_timeout(
    proxy_url: Option<&str>,
    timeout: Duration,
) -> Result<Client, Box<dyn Error>> {
    let mut builder = Client::builder()
        .user_agent(APP_HTTP_USER_AGENT)
        .timeout(timeout);
    if let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }
    Ok(builder.build()?)
}

pub const PROXY_SCHEMES_ALL: &[&str] = &[
    "http://",
    "https://",
    "socks4://",
    "socks4a://",
    "socks5://",
    "socks5h://",
];
pub const PROXY_SCHEMES_HTTP_ONLY: &[&str] = &["http://", "https://"];

pub fn engine_proxy_allowed_schemes(engine: EngineKind) -> &'static [&'static str] {
    match engine {
        EngineKind::Aria2 => PROXY_SCHEMES_HTTP_ONLY,
        EngineKind::YtDlp => PROXY_SCHEMES_ALL,
        EngineKind::QBittorrent => PROXY_SCHEMES_ALL,
    }
}

fn format_proxy_schemes(allowed: &[&str]) -> String {
    allowed
        .iter()
        .map(|prefix| prefix.trim_end_matches("://"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn validate_proxy_url(value: &str, allowed_schemes: &[&str]) -> Result<(), Box<dyn Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let lower = trimmed.to_ascii_lowercase();
    if !allowed_schemes
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return Err(format!(
            "proxy url scheme must be one of: {}",
            format_proxy_schemes(allowed_schemes)
        )
        .into());
    }
    reqwest::Proxy::all(trimmed)
        .map_err(|error| -> Box<dyn Error> { format!("invalid proxy url: {error}").into() })?;
    Ok(())
}

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

    pub fn download_file_conflict(
        &self,
        input: CreateDownloadTaskInput,
    ) -> Result<Option<DownloadFileConflict>, Box<dyn Error>> {
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

        local_download_file_conflict(&input, &engine_settings)
    }

    pub fn download_duplicate_check(
        &self,
        input: CreateDownloadTaskInput,
    ) -> Result<DownloadDuplicateCheck, Box<dyn Error>> {
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

        let local_file_conflict = local_download_file_conflict(&input, &engine_settings)?;
        let mut matches = Vec::new();
        let mut http_identity_cache = HashMap::new();
        let input_source = normalize_duplicate_source(&input.source);
        let input_save_path = normalize_duplicate_path(&input.save_path);
        let input_file_name = input.file_name.trim();
        let input_http_identity = download_http_resource_identity(
            input.source_type,
            &input.source,
            &engine_settings,
            &mut http_identity_cache,
        );
        let input_total_bytes = input_http_identity
            .as_ref()
            .and_then(|identity| identity.total_bytes);
        let input_torrent_info_hash = input_torrent_info_hash(&input)?;

        for task in self.repository.list_created_desc()? {
            let Some(task_state) = duplicate_task_state(task.status) else {
                continue;
            };

            let same_source = normalize_duplicate_source(&task.source) == input_source;
            if same_source {
                push_duplicate_match(
                    &mut matches,
                    DownloadDuplicateKind::SameSource,
                    &task,
                    task_state,
                );
            }

            if !same_source
                && same_final_url(
                    input_http_identity.as_ref(),
                    input_file_name,
                    &task,
                    &engine_settings,
                    &mut http_identity_cache,
                )
            {
                push_duplicate_match(
                    &mut matches,
                    DownloadDuplicateKind::SameFinalUrl,
                    &task,
                    task_state,
                );
            }

            if normalize_duplicate_path(&task.save_path) == input_save_path
                && task.file_name.trim() == input_file_name
            {
                push_duplicate_match(
                    &mut matches,
                    DownloadDuplicateKind::SameSavePath,
                    &task,
                    task_state,
                );
            }

            if let Some(total_bytes) = input_total_bytes.filter(|value| *value > 0) {
                if task.file_name.trim() == input_file_name && task.total_bytes == total_bytes {
                    push_duplicate_match(
                        &mut matches,
                        DownloadDuplicateKind::SameNameAndSize,
                        &task,
                        task_state,
                    );
                }
            }

            if let Some(info_hash) = input_torrent_info_hash.as_deref() {
                if task_torrent_info_hash(&task).as_deref() == Some(info_hash) {
                    push_duplicate_match(
                        &mut matches,
                        DownloadDuplicateKind::SameTorrentInfoHash,
                        &task,
                        task_state,
                    );
                }
            }
        }

        Ok(DownloadDuplicateCheck {
            matches,
            local_file_conflict,
        })
    }

    pub fn create(
        &self,
        mut input: CreateDownloadTaskInput,
    ) -> Result<DownloadTask, Box<dyn Error>> {
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

        let conflict_action = input
            .file_conflict_action
            .unwrap_or(FileConflictAction::Prompt);
        if let Some(conflict) = local_download_file_conflict(&input, &engine_settings)? {
            match conflict_action {
                FileConflictAction::Prompt => {
                    return Err(download_file_conflict_message(&conflict).into());
                }
                FileConflictAction::Overwrite => {
                    remove_conflicting_download_file(Path::new(&conflict.path))?;
                }
                FileConflictAction::Rename => {
                    input.file_name = available_download_file_name(&input, &engine_settings)?;
                }
            }
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
            http_referrer: input.http_referrer,
        };
        self.repository.create(&id, &task_input)?;
        let task = self.repository.get_by_id(&id)?;

        if is_local_download_engine(task.engine) && !self.has_local_download_capacity()? {
            return Ok(task);
        }

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
                DownloadStatus::Queued
                    | DownloadStatus::Completed
                    | DownloadStatus::Deleted
                    | DownloadStatus::Failed
            ) || task.engine_task_id.is_none()
            {
                continue;
            }

            if let Err(error) = self.refresh_one(&task) {
                logger::warn(format!(
                    "download task refresh skipped: task_id={}, error={error}",
                    task.id
                ));
            }
        }

        self.start_queued_local_tasks()?;

        self.repository.list_created_desc()
    }

    pub fn pause_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        self.pause_tasks_internal(ids, true)
    }

    fn pause_tasks_internal(
        &self,
        ids: &[String],
        start_next_after_pause: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut should_start_next = false;
        for task in self.repository.list_by_ids(ids)? {
            if !matches!(
                task.status,
                DownloadStatus::Queued | DownloadStatus::Running
            ) {
                continue;
            }
            if task.status == DownloadStatus::Queued {
                self.repository.update_engine_state_if_current(
                    &task.id,
                    task.status,
                    task.engine_task_id.as_deref(),
                    DownloadStatus::Paused,
                    task.progress,
                    0,
                    task.downloaded_bytes,
                    task.total_bytes,
                    task.engine_task_id.as_deref(),
                    None,
                )?;
                continue;
            }
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            match engine_adapters::pause_task(&engine_settings, &task) {
                Ok(state) => {
                    let status = state.status;
                    if self.apply_engine_state_if_current(&task, state)?
                        && matches!(status, DownloadStatus::Paused | DownloadStatus::Failed)
                    {
                        should_start_next = true;
                    }
                }
                Err(error) => {
                    if self.repository.mark_failed_if_current(
                        &task.id,
                        task.status,
                        task.engine_task_id.as_deref(),
                        &error.to_string(),
                    )? {
                        if start_next_after_pause {
                            self.start_queued_local_tasks()?;
                        }
                    }
                    return Err(error);
                }
            }
        }
        if should_start_next && start_next_after_pause {
            self.start_queued_local_tasks()?;
        }
        Ok(())
    }

    pub fn resume_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            if !matches!(task.status, DownloadStatus::Paused | DownloadStatus::Failed) {
                continue;
            }
            if is_local_download_engine(task.engine) && !self.has_local_download_capacity()? {
                self.queue_local_task(&task)?;
                continue;
            }
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            match engine_adapters::resume_task(&engine_settings, &task, self.database_path.clone())
            {
                Ok(state) => {
                    self.apply_engine_state_if_current(&task, state)?;
                }
                Err(error) => {
                    self.repository.mark_failed_if_current(
                        &task.id,
                        task.status,
                        task.engine_task_id.as_deref(),
                        &error.to_string(),
                    )?;
                    self.start_queued_local_tasks()?;
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

    pub fn open_download_directory(&self, id: &str) -> Result<(), Box<dyn Error>> {
        let task = self.repository.get_by_id(id)?;
        if task.engine != EngineKind::Aria2 && task.engine != EngineKind::YtDlp {
            return Err(
                "download task does not use a local download directory managed by UniDL".into(),
            );
        }

        let path = Path::new(&task.save_path);
        if !fs::metadata(path)?.is_dir() {
            return Err("download path is not a directory".into());
        }

        open_path(path)
    }

    pub fn delete_tasks(
        &self,
        ids: &[String],
        delete_completed_files: bool,
    ) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            let delete_downloaded = delete_completed_files
                && (task.engine != EngineKind::QBittorrent
                    || downloaded_entry_path(&task).exists());
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
                    self.repository.mark_failed_if_current(
                        &task.id,
                        task.status,
                        task.engine_task_id.as_deref(),
                        &error.to_string(),
                    )?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn clear_download_records(
        &self,
        older_than_days: Option<i64>,
    ) -> Result<usize, Box<dyn Error>> {
        if matches!(older_than_days, Some(days) if days <= 0) {
            return Err("older_than_days must be greater than 0".into());
        }

        Ok(self.repository.clear_download_records(older_than_days)?)
    }

    pub fn pause_all_unfinished(&self) -> Result<(), Box<dyn Error>> {
        let ids = self
            .repository
            .list_unfinished()?
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        self.pause_tasks_internal(&ids, false)
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
        self.apply_engine_state_if_current(task, state)?;
        Ok(())
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
        self.apply_engine_state_side_effects(task_id, &state)
    }

    fn apply_engine_state_if_current(
        &self,
        task: &DownloadTask,
        state: engine_adapters::EngineTaskState,
    ) -> Result<bool, Box<dyn Error>> {
        let updated = self.repository.update_engine_state_if_current(
            &task.id,
            task.status,
            task.engine_task_id.as_deref(),
            state.status,
            state.progress,
            state.speed_bytes_per_sec,
            state.downloaded_bytes,
            state.total_bytes,
            state.engine_task_id.as_deref(),
            state.error_message.as_deref(),
        )?;
        if updated {
            self.apply_engine_state_side_effects(&task.id, &state)?;
        }
        Ok(updated)
    }

    fn apply_engine_state_side_effects(
        &self,
        task_id: &str,
        state: &engine_adapters::EngineTaskState,
    ) -> Result<(), Box<dyn Error>> {
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
        if matches!(
            state.status,
            DownloadStatus::Completed | DownloadStatus::Failed
        ) {
            self.start_queued_local_tasks()?;
        }
        Ok(())
    }

    fn has_local_download_capacity(&self) -> Result<bool, Box<dyn Error>> {
        let limit = self.local_download_concurrency()?;
        Ok(self.repository.count_running_local_downloads()? < limit)
    }

    fn local_download_concurrency(&self) -> Result<i64, Box<dyn Error>> {
        let value = AppSettingsRepository::new(self.connection)
            .get()?
            .local_download_concurrency;
        if value <= 0 {
            return Err("local download concurrency must be greater than 0".into());
        }
        Ok(value)
    }

    fn queue_local_task(&self, task: &DownloadTask) -> Result<bool, Box<dyn Error>> {
        let next_engine_task_id = if task.status == DownloadStatus::Failed {
            None
        } else {
            task.engine_task_id.as_deref()
        };
        Ok(self.repository.update_engine_state_if_current(
            &task.id,
            task.status,
            task.engine_task_id.as_deref(),
            DownloadStatus::Queued,
            task.progress,
            0,
            task.downloaded_bytes,
            task.total_bytes,
            next_engine_task_id,
            None,
        )?)
    }

    pub fn start_queued_local_tasks(&self) -> Result<(), Box<dyn Error>> {
        let limit = self.local_download_concurrency()?;
        loop {
            let available = limit - self.repository.count_running_local_downloads()?;
            if available <= 0 {
                return Ok(());
            }

            let tasks = self.repository.list_queued_local_oldest(available)?;
            if tasks.is_empty() {
                return Ok(());
            }

            for task in tasks {
                let engine_settings =
                    EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
                let state = match self.start_queued_local_task(&engine_settings, &task) {
                    Ok(state) => state,
                    Err(error) => {
                        let message = error.to_string();
                        self.repository.mark_failed_if_current(
                            &task.id,
                            task.status,
                            task.engine_task_id.as_deref(),
                            &message,
                        )?;
                        logger::warn(format!(
                            "queued local download start failed: task_id={}, error={message}",
                            task.id
                        ));
                        continue;
                    }
                };
                self.apply_engine_state_if_current(&task, state)?;
            }
        }
    }

    fn start_queued_local_task(
        &self,
        engine_settings: &EngineSettings,
        task: &DownloadTask,
    ) -> Result<engine_adapters::EngineTaskState, Box<dyn Error>> {
        if !engine_settings.enabled {
            return Err(format!("{} is disabled", engine_settings.id).into());
        }
        if !engine_settings
            .supported_source_types
            .contains(&task.source_type)
        {
            return Err(format!(
                "{} does not support {} tasks",
                engine_settings.id,
                task.source_type.as_db()
            )
            .into());
        }

        if task.engine_task_id.is_some() || task.progress > 0.0 || task.downloaded_bytes > 0 {
            let mut resumable_task = task.clone();
            if resumable_task.engine_task_id.is_some() {
                resumable_task.status = DownloadStatus::Paused;
            }
            return engine_adapters::resume_task(
                engine_settings,
                &resumable_task,
                self.database_path.clone(),
            );
        }

        engine_adapters::add_task(engine_settings, task, self.database_path.clone())
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
        for partial_path in downloaded_partial_entry_paths(task, &path) {
            remove_downloaded_entry(&partial_path)?;
        }
    }
    remove_downloaded_entry(&path)
}

fn local_download_file_conflict(
    input: &CreateDownloadTaskInput,
    settings: &EngineSettings,
) -> Result<Option<DownloadFileConflict>, Box<dyn Error>> {
    if matches!(settings.engine, EngineKind::QBittorrent) {
        return Ok(None);
    }

    match input.source_type {
        SourceType::Http | SourceType::Ftp => local_single_file_conflict(input, settings),
        SourceType::Magnet | SourceType::Torrent => Ok(None),
    }
}

#[derive(Clone)]
struct DownloadHttpResourceIdentity {
    final_url: String,
    total_bytes: Option<i64>,
}

fn duplicate_task_state(status: DownloadStatus) -> Option<DownloadDuplicateTaskState> {
    match status {
        DownloadStatus::Completed => Some(DownloadDuplicateTaskState::Completed),
        DownloadStatus::Queued
        | DownloadStatus::Running
        | DownloadStatus::Paused
        | DownloadStatus::Failed => Some(DownloadDuplicateTaskState::Active),
        DownloadStatus::Deleted => None,
    }
}

fn push_duplicate_match(
    matches: &mut Vec<DownloadDuplicateMatch>,
    kind: DownloadDuplicateKind,
    task: &DownloadTask,
    task_state: DownloadDuplicateTaskState,
) {
    matches.push(DownloadDuplicateMatch {
        kind,
        task: task.clone(),
        task_state,
    });
}

fn normalize_duplicate_source(value: &str) -> String {
    value.trim().to_string()
}

fn normalize_duplicate_path(value: &str) -> String {
    let mut value = value.trim().replace('\\', "/");
    while value.len() > 1 && value.ends_with('/') {
        value.pop();
    }
    value
}

fn download_http_resource_identity(
    source_type: SourceType,
    source: &str,
    settings: &EngineSettings,
    cache: &mut HashMap<String, Option<DownloadHttpResourceIdentity>>,
) -> Option<DownloadHttpResourceIdentity> {
    if source_type != SourceType::Http {
        return None;
    }

    let source = source.trim();
    if source.is_empty() {
        return None;
    }

    let cache_key = normalize_duplicate_source(source);
    if let Some(identity) = cache.get(&cache_key) {
        return identity.clone();
    }

    let identity = fetch_http_resource_identity(source, settings)
        .map_err(|error| {
            logger::warn(format!(
                "download duplicate final url check skipped: source={source}, error={error}"
            ));
            error
        })
        .ok();
    cache.insert(cache_key, identity.clone());
    identity
}

fn fetch_http_resource_identity(
    source: &str,
    settings: &EngineSettings,
) -> Result<DownloadHttpResourceIdentity, Box<dyn Error>> {
    let client = build_app_http_client_with_timeout(
        engine_adapters::engine_proxy_url(settings),
        DUPLICATE_HTTP_METADATA_TIMEOUT,
    )?;
    let response = client.head(source).send()?;
    if !response.status().is_success() {
        return Err(format!("http metadata status failed: {}", response.status()).into());
    }

    let total_bytes = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|value| *value > 0);

    Ok(DownloadHttpResourceIdentity {
        final_url: normalize_duplicate_source(response.url().as_str()),
        total_bytes,
    })
}

fn same_final_url(
    input_identity: Option<&DownloadHttpResourceIdentity>,
    input_file_name: &str,
    task: &DownloadTask,
    settings: &EngineSettings,
    cache: &mut HashMap<String, Option<DownloadHttpResourceIdentity>>,
) -> bool {
    let Some(input_identity) = input_identity else {
        return false;
    };
    if input_identity.final_url.is_empty() {
        return false;
    }

    if normalize_duplicate_source(&task.source) == input_identity.final_url {
        return true;
    }

    if task.source_type != SourceType::Http || task.file_name.trim() != input_file_name {
        return false;
    }

    download_http_resource_identity(task.source_type, &task.source, settings, cache)
        .map(|task_identity| task_identity.final_url == input_identity.final_url)
        .unwrap_or(false)
}

fn input_torrent_info_hash(
    input: &CreateDownloadTaskInput,
) -> Result<Option<String>, Box<dyn Error>> {
    match input.source_type {
        SourceType::Magnet => Ok(parse_magnet_info_hash(&input.source)),
        SourceType::Torrent => Ok(Some(crate::torrent_metadata::read_torrent_info_hash(
            input.source.trim(),
        )?)),
        SourceType::Http | SourceType::Ftp => Ok(None),
    }
}

fn task_torrent_info_hash(task: &DownloadTask) -> Option<String> {
    match task.source_type {
        SourceType::Magnet => parse_magnet_info_hash(&task.source),
        SourceType::Torrent => crate::torrent_metadata::read_torrent_info_hash(task.source.trim())
            .map_err(|error| {
                logger::warn(format!(
                    "download duplicate torrent hash check skipped: task_id={}, error={error}",
                    task.id
                ));
                error
            })
            .ok(),
        SourceType::Http | SourceType::Ftp => None,
    }
}

fn parse_magnet_info_hash(source: &str) -> Option<String> {
    let (_, query) = source.trim().split_once('?')?;
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        if !key.eq_ignore_ascii_case("xt") {
            return None;
        }
        let value = percent_decode_query_value(value)?;
        let value = strip_btih_prefix(&value)?;
        normalize_btih_hash(value)
    })
}

fn strip_btih_prefix(value: &str) -> Option<&str> {
    let prefix = "urn:btih:";
    let bytes = value.as_bytes();
    if bytes.len() < prefix.len() || !bytes[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    {
        return None;
    }
    Some(&value[prefix.len()..])
}

fn percent_decode_query_value(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes.get(index + 1).and_then(|byte| hex_value(*byte))?;
            let low = bytes.get(index + 2).and_then(|byte| hex_value(*byte))?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn normalize_btih_hash(value: &str) -> Option<String> {
    let value = value.trim();
    if value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Some(value.to_ascii_lowercase());
    }
    if value.len() == 32 {
        return base32_btih_to_hex(value);
    }
    None
}

fn base32_btih_to_hex(value: &str) -> Option<String> {
    let mut bits = 0;
    let mut buffer = 0_u64;
    let mut bytes = Vec::with_capacity(20);

    for byte in value.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            b'2'..=b'7' => byte - b'2' + 26,
            _ => return None,
        };
        buffer = (buffer << 5) | u64::from(value);
        bits += 5;
        while bits >= 8 {
            bits -= 8;
            bytes.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    if bytes.len() != 20 {
        return None;
    }

    Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn is_local_download_engine(engine: EngineKind) -> bool {
    matches!(engine, EngineKind::Aria2 | EngineKind::YtDlp)
}

fn local_single_file_conflict(
    input: &CreateDownloadTaskInput,
    settings: &EngineSettings,
) -> Result<Option<DownloadFileConflict>, Box<dyn Error>> {
    let save_path = Path::new(input.save_path.trim());
    let file_name = local_single_file_name(input, settings);
    let path = save_path.join(&file_name);
    if path.exists() {
        return Ok(Some(download_file_conflict(file_name, path)));
    }

    if settings.engine == EngineKind::YtDlp && Path::new(&file_name).extension().is_none() {
        if let Some(path) = resolve_ytdlp_downloaded_entry_path(save_path, &file_name) {
            return Ok(Some(download_file_conflict(
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or(file_name),
                path,
            )));
        }
    }

    Ok(None)
}

fn local_single_file_name(input: &CreateDownloadTaskInput, settings: &EngineSettings) -> String {
    if settings.engine == EngineKind::YtDlp {
        engine_adapters::sanitize_ytdlp_output_name(input.file_name.trim())
    } else {
        input.file_name.trim().to_string()
    }
}

fn download_file_conflict(file_name: String, path: PathBuf) -> DownloadFileConflict {
    DownloadFileConflict {
        file_name,
        path: path.to_string_lossy().into_owned(),
    }
}

fn download_file_conflict_message(conflict: &DownloadFileConflict) -> String {
    format!("download file already exists: {}", conflict.path)
}

fn remove_conflicting_download_file(path: &Path) -> Result<(), Box<dyn Error>> {
    remove_conflicting_file(path)?;
    remove_conflicting_file(&downloaded_partial_entry_path(path))?;
    let mut aria2_control_path = path.as_os_str().to_os_string();
    aria2_control_path.push(".aria2");
    remove_conflicting_file(&PathBuf::from(aria2_control_path))
}

fn remove_conflicting_file(path: &Path) -> Result<(), Box<dyn Error>> {
    match fs::remove_file(path) {
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

fn available_download_file_name(
    input: &CreateDownloadTaskInput,
    settings: &EngineSettings,
) -> Result<String, Box<dyn Error>> {
    if !matches!(input.source_type, SourceType::Http | SourceType::Ftp) {
        return Err("auto rename is only available for single-file downloads".into());
    }

    let original = input.file_name.trim();
    let path = Path::new(original);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(original);
    let extension = path.extension().and_then(|value| value.to_str());

    for index in 1..10_000 {
        let candidate = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem} ({index}).{extension}"),
            _ => format!("{stem} ({index})"),
        };
        let mut next_input = input.clone();
        next_input.file_name = candidate.clone();
        if local_download_file_conflict(&next_input, settings)?.is_none() {
            return Ok(candidate);
        }
    }

    Err("no available download file name".into())
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
        Err(error)
            if error.kind() == io::ErrorKind::IsADirectory
                || (error.kind() == io::ErrorKind::PermissionDenied && path.is_dir()) =>
        {
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

pub(crate) fn downloaded_entry_path(task: &DownloadTask) -> PathBuf {
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

fn downloaded_partial_entry_paths(task: &DownloadTask, path: &Path) -> Vec<PathBuf> {
    let mut partial_paths = Vec::with_capacity(2);
    partial_paths.push(downloaded_partial_entry_path(path));
    if task.engine == EngineKind::Aria2 {
        let mut control_path = path.as_os_str().to_os_string();
        control_path.push(".aria2");
        partial_paths.push(PathBuf::from(control_path));
    }
    partial_paths
}

fn open_path(path: &Path) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        engine_adapters::hide_console_window(&mut command);
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
    if input.engine != EngineKind::QBittorrent && input.save_path.trim().is_empty() {
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
        if input.theme_mode != "light" && input.theme_mode != "dark" {
            return Err("theme mode must be light or dark".into());
        }
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
        validate_proxy_url(&input.app_proxy_url, PROXY_SCHEMES_ALL)?;
        if input.local_download_concurrency <= 0 {
            return Err("local download concurrency must be greater than 0".into());
        }
        if input.auto_clean_download_tasks_days <= 0 {
            return Err("auto cleanup days must be greater than 0".into());
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
        let app_proxy_url = AppSettingsRepository::new(self.repository.connection())
            .get()
            .ok()
            .map(|settings| settings.app_proxy_url)
            .filter(|value| !value.trim().is_empty());
        let installed = engine_install::install_latest(current.engine, app_proxy_url.as_deref())?;
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
            proxy_url: current.proxy_url,
            user_agent: current.user_agent,
            speed_limit_bytes_per_sec: current.speed_limit_bytes_per_sec,
            qbittorrent_download_limit_bytes_per_sec: current
                .qbittorrent_download_limit_bytes_per_sec,
            qbittorrent_upload_limit_bytes_per_sec: current.qbittorrent_upload_limit_bytes_per_sec,
            qbittorrent_seed_ratio_limit: current.qbittorrent_seed_ratio_limit,
            qbittorrent_seed_time_limit_minutes: current.qbittorrent_seed_time_limit_minutes,
            aria2_enable_dht: current.aria2_enable_dht,
            aria2_enable_dht6: current.aria2_enable_dht6,
            aria2_enable_peer_exchange: current.aria2_enable_peer_exchange,
            aria2_enable_lpd: current.aria2_enable_lpd,
            aria2_bt_listen_port: current.aria2_bt_listen_port,
            aria2_bt_max_peers: current.aria2_bt_max_peers,
            aria2_max_connection_per_server: current.aria2_max_connection_per_server,
            aria2_split: current.aria2_split,
            aria2_min_split_size: current.aria2_min_split_size,
            aria2_file_allocation: current.aria2_file_allocation,
            aria2_seed_time: current.aria2_seed_time,
            aria2_seed_ratio: current.aria2_seed_ratio,
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
            proxy_url: input.proxy_url,
            user_agent: input.user_agent,
            speed_limit_bytes_per_sec: input.speed_limit_bytes_per_sec,
            qbittorrent_download_limit_bytes_per_sec: input
                .qbittorrent_download_limit_bytes_per_sec,
            qbittorrent_upload_limit_bytes_per_sec: input.qbittorrent_upload_limit_bytes_per_sec,
            qbittorrent_seed_ratio_limit: input.qbittorrent_seed_ratio_limit,
            qbittorrent_seed_time_limit_minutes: input.qbittorrent_seed_time_limit_minutes,
            aria2_enable_dht: input.aria2_enable_dht,
            aria2_enable_dht6: input.aria2_enable_dht6,
            aria2_enable_peer_exchange: input.aria2_enable_peer_exchange,
            aria2_enable_lpd: input.aria2_enable_lpd,
            aria2_bt_listen_port: input.aria2_bt_listen_port,
            aria2_bt_max_peers: input.aria2_bt_max_peers,
            aria2_max_connection_per_server: input.aria2_max_connection_per_server,
            aria2_split: input.aria2_split,
            aria2_min_split_size: input.aria2_min_split_size,
            aria2_file_allocation: input.aria2_file_allocation,
            aria2_seed_time: input.aria2_seed_time,
            aria2_seed_ratio: input.aria2_seed_ratio,
            priority: input.priority,
            updated_at: String::new(),
        };
        crate::engine_adapters::test_connection(&settings)
    }

    pub fn update_tracker_subscription(
        &self,
        settings_id: &str,
        subscription_urls: &str,
    ) -> Result<EngineSettings, Box<dyn Error>> {
        let current = self.repository.get(settings_id)?;
        if current.engine != EngineKind::Aria2 {
            return Err("tracker subscription is only supported for aria2".into());
        }

        let subscription_urls = parse_tracker_subscription_urls(subscription_urls);
        if subscription_urls.is_empty() {
            return Err("tracker subscription url is required".into());
        }

        let app_proxy_url = AppSettingsRepository::new(self.repository.connection())
            .get()
            .ok()
            .map(|settings| settings.app_proxy_url)
            .filter(|value| !value.trim().is_empty());
        let trackers = fetch_trackers(&subscription_urls, app_proxy_url.as_deref())?;
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
            tracker_subscription_url: Some(subscription_urls.join("\n")),
            trackers,
            proxy_url: current.proxy_url,
            user_agent: current.user_agent,
            speed_limit_bytes_per_sec: current.speed_limit_bytes_per_sec,
            qbittorrent_download_limit_bytes_per_sec: current
                .qbittorrent_download_limit_bytes_per_sec,
            qbittorrent_upload_limit_bytes_per_sec: current.qbittorrent_upload_limit_bytes_per_sec,
            qbittorrent_seed_ratio_limit: current.qbittorrent_seed_ratio_limit,
            qbittorrent_seed_time_limit_minutes: current.qbittorrent_seed_time_limit_minutes,
            aria2_enable_dht: current.aria2_enable_dht,
            aria2_enable_dht6: current.aria2_enable_dht6,
            aria2_enable_peer_exchange: current.aria2_enable_peer_exchange,
            aria2_enable_lpd: current.aria2_enable_lpd,
            aria2_bt_listen_port: current.aria2_bt_listen_port,
            aria2_bt_max_peers: current.aria2_bt_max_peers,
            aria2_max_connection_per_server: current.aria2_max_connection_per_server,
            aria2_split: current.aria2_split,
            aria2_min_split_size: current.aria2_min_split_size,
            aria2_file_allocation: current.aria2_file_allocation,
            aria2_seed_time: current.aria2_seed_time,
            aria2_seed_ratio: current.aria2_seed_ratio,
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

    let proxy_url = input.proxy_url.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    if let Some(proxy_url) = proxy_url.as_deref() {
        validate_proxy_url(proxy_url, engine_proxy_allowed_schemes(input.engine))?;
    }
    let user_agent = input.user_agent.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    });
    if user_agent
        .as_deref()
        .is_some_and(|value| value.contains(['\r', '\n']))
    {
        return Err("user agent cannot contain line breaks".into());
    }
    if input.speed_limit_bytes_per_sec < 0 {
        return Err("speed limit cannot be negative".into());
    }
    if input.qbittorrent_download_limit_bytes_per_sec < 0 {
        return Err("qBittorrent download limit cannot be negative".into());
    }
    if input.qbittorrent_upload_limit_bytes_per_sec < 0 {
        return Err("qBittorrent upload limit cannot be negative".into());
    }
    if !input.qbittorrent_seed_ratio_limit.is_finite() || input.qbittorrent_seed_ratio_limit < 0.0 {
        return Err("qBittorrent seed ratio limit cannot be negative".into());
    }
    if input.qbittorrent_seed_time_limit_minutes < 0 {
        return Err("qBittorrent seed time limit cannot be negative".into());
    }

    if input.aria2_bt_listen_port < 1 || input.aria2_bt_listen_port > 65_535 {
        return Err("aria2 BT listen port must be between 1 and 65535".into());
    }
    if input.aria2_bt_max_peers < 0 {
        return Err("aria2 BT max peers cannot be negative".into());
    }
    if input.aria2_max_connection_per_server < 1 {
        return Err("aria2 max connection per server must be at least 1".into());
    }
    if input.aria2_split < 1 {
        return Err("aria2 split must be at least 1".into());
    }
    let aria2_min_split_size = input.aria2_min_split_size.trim().to_string();
    if aria2_min_split_size.is_empty()
        || aria2_min_split_size
            .chars()
            .any(|character| character.is_whitespace())
    {
        return Err("aria2 min split size is required and cannot contain spaces".into());
    }
    let aria2_file_allocation = input.aria2_file_allocation.trim().to_ascii_lowercase();
    if !matches!(
        aria2_file_allocation.as_str(),
        "none" | "prealloc" | "trunc" | "falloc"
    ) {
        return Err("aria2 file allocation must be one of none, prealloc, trunc, falloc".into());
    }
    if input.aria2_seed_time < 0 {
        return Err("aria2 seed time cannot be negative".into());
    }
    if !input.aria2_seed_ratio.is_finite() || input.aria2_seed_ratio < 0.0 {
        return Err("aria2 seed ratio cannot be negative".into());
    }

    Ok(EngineSettingsInput {
        name,
        tracker_subscription_url: input.tracker_subscription_url.and_then(|value| {
            match parse_tracker_subscription_urls(&value) {
                urls if urls.is_empty() => None,
                urls => Some(urls.join("\n")),
            }
        }),
        trackers: normalize_trackers(input.trackers),
        supported_source_types,
        proxy_url,
        user_agent,
        aria2_min_split_size,
        aria2_file_allocation,
        ..input
    })
}

fn fetch_trackers(
    subscription_urls: &[String],
    app_proxy_url: Option<&str>,
) -> Result<Vec<String>, Box<dyn Error>> {
    if subscription_urls.is_empty() {
        return Err("tracker subscription url is required".into());
    }

    let client = build_app_http_client(app_proxy_url)?;
    let mut trackers = Vec::new();
    for url in subscription_urls {
        let response = client.get(url).send()?;
        if !response.status().is_success() {
            return Err(
                format!("tracker subscription failed: {url}: {}", response.status()).into(),
            );
        }
        trackers.extend(parse_trackers(&response.text()?));
    }

    Ok(normalize_trackers(trackers))
}

fn parse_tracker_subscription_urls(value: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for url in value
        .split(|character: char| character.is_whitespace() || character == ',' || character == ';')
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        if !urls.iter().any(|item: &String| item == url) {
            urls.push(url.to_string());
        }
    }
    urls
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
        sync::{mpsc, Arc, Mutex},
        thread,
        time::Duration,
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
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: true,
                aria2_enable_dht6: true,
                aria2_enable_peer_exchange: true,
                aria2_enable_lpd: true,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
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
                tracker_subscription_url: saved.tracker_subscription_url,
                trackers: saved.trackers,
                proxy_url: saved.proxy_url,
                user_agent: saved.user_agent,
                speed_limit_bytes_per_sec: saved.speed_limit_bytes_per_sec,
                qbittorrent_download_limit_bytes_per_sec: saved
                    .qbittorrent_download_limit_bytes_per_sec,
                qbittorrent_upload_limit_bytes_per_sec: saved
                    .qbittorrent_upload_limit_bytes_per_sec,
                qbittorrent_seed_ratio_limit: saved.qbittorrent_seed_ratio_limit,
                qbittorrent_seed_time_limit_minutes: saved.qbittorrent_seed_time_limit_minutes,
                aria2_enable_dht: saved.aria2_enable_dht,
                aria2_enable_dht6: saved.aria2_enable_dht6,
                aria2_enable_peer_exchange: saved.aria2_enable_peer_exchange,
                aria2_enable_lpd: saved.aria2_enable_lpd,
                aria2_bt_listen_port: saved.aria2_bt_listen_port,
                aria2_bt_max_peers: saved.aria2_bt_max_peers,
                aria2_max_connection_per_server: saved.aria2_max_connection_per_server,
                aria2_split: saved.aria2_split,
                aria2_min_split_size: saved.aria2_min_split_size,
                aria2_file_allocation: saved.aria2_file_allocation,
                aria2_seed_time: saved.aria2_seed_time,
                aria2_seed_ratio: saved.aria2_seed_ratio,
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
    fn tracker_subscription_urls_are_split_and_deduplicated() {
        let urls = parse_tracker_subscription_urls(
            " https://one.example/trackers.txt\nhttps://two.example/list.txt;https://one.example/trackers.txt, ",
        );

        assert_eq!(
            urls,
            vec![
                "https://one.example/trackers.txt".to_string(),
                "https://two.example/list.txt".to_string(),
            ]
        );
    }

    #[test]
    fn tracker_update_merges_multiple_subscriptions_and_deduplicates_trackers() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_unreachable_aria2_settings(&connection);
        let (subscription_urls, server) = start_fake_tracker_subscriptions(vec![
            "udp://tracker.one:80/announce\nudp://tracker.two:80/announce\n",
            "UDP://TRACKER.ONE:80/announce\nhttps://tracker.three/announce\n",
        ]);

        let service = EngineSettingsService::new(&connection);
        let saved = service
            .update_tracker_subscription("aria2", &subscription_urls.join("\n"))
            .expect("trackers should update");

        assert_eq!(
            saved.tracker_subscription_url,
            Some(subscription_urls.join("\n"))
        );
        assert_eq!(
            saved.trackers,
            vec![
                "udp://tracker.one:80/announce".to_string(),
                "udp://tracker.two:80/announce".to_string(),
                "https://tracker.three/announce".to_string(),
            ]
        );

        server.join().expect("fake tracker server should finish");
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
                theme_mode: "light".to_string(),
                web_access_enabled: false,
                web_access_password: String::new(),
                web_access_url: "http://127.0.0.1:18080".to_string(),
                private_download_domains: vec!["example.test".to_string()],
                app_proxy_url: String::new(),
                torrent_file_association_enabled: false,
                auto_start_enabled: false,
                auto_start_minimized_to_tray: false,
                close_to_tray_enabled: false,
                download_completion_notification_enabled: false,
                prevent_sleep_when_downloading_enabled: false,
                prevent_sleep_when_web_access_enabled: false,
                local_download_concurrency: 5,
                auto_clean_download_tasks_enabled: false,
                auto_clean_download_tasks_days: 365,
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
                    file_name: None,
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
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: false,
                aria2_enable_dht6: false,
                aria2_enable_peer_exchange: false,
                aria2_enable_lpd: false,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
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
    fn deleting_unfinished_aria2_task_removes_partial_and_control_files() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, server) = start_fake_aria2_status("active", 2);
        save_aria2_settings_with_url(&connection, url);

        let download_dir = temp_download_dir();
        fs::create_dir_all(&download_dir).expect("download dir should create");
        let download_file = download_dir.join("movie.mkv");
        let control_file = download_dir.join("movie.mkv.aria2");
        fs::write(&download_file, b"partial").expect("download file should write");
        fs::write(&control_file, b"control").expect("control file should write");

        insert_aria2_task(
            &connection,
            "running-aria2-remove-files-task",
            DownloadStatus::Running,
            download_dir.to_string_lossy().as_ref(),
            "movie.mkv",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["running-aria2-remove-files-task".to_string()], true)
            .expect("unfinished aria2 task should delete partial and control files");

        server.join().expect("fake aria2 should finish");
        assert!(!download_file.exists());
        assert!(!control_file.exists());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
        let _ = fs::remove_dir_all(download_dir);
    }

    #[test]
    fn deleting_unfinished_aria2_task_removes_download_folder_when_requested() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, server) = start_fake_aria2_status("active", 2);
        save_aria2_settings_with_url(&connection, url);

        let download_dir = temp_download_dir();
        let download_folder = download_dir.join("album");
        fs::create_dir_all(&download_folder).expect("download folder should create");
        fs::write(download_folder.join("track.bin"), b"partial")
            .expect("download folder file should write");

        insert_aria2_task(
            &connection,
            "running-aria2-remove-folder-task",
            DownloadStatus::Running,
            download_dir.to_string_lossy().as_ref(),
            "album",
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        service
            .delete_tasks(&["running-aria2-remove-folder-task".to_string()], true)
            .expect("unfinished aria2 task should delete download folder when requested");

        server.join().expect("fake aria2 should finish");
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
            http_referrer: None,
            file_conflict_action: None,
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
                    http_referrer: None,
                },
            )
            .expect("failed task should insert");
        repository
            .mark_failed_if_current(
                "failed-without-engine-id",
                DownloadStatus::Queued,
                None,
                "engine failed before id",
            )
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
    fn clear_download_records_removes_only_finished_records_by_age() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");

        let insert_task =
            |id: &str, status: DownloadStatus, created_at: &str, completed_at: Option<&str>| {
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
                        save_path,
                        created_at,
                        completed_at
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                    "#,
                        (
                            id,
                            SourceType::Http.as_db(),
                            "http://example.test/file.bin",
                            "aria2",
                            EngineKind::Aria2.as_db(),
                            "gid",
                            "file.bin",
                            status.as_db(),
                            "C:\\Downloads",
                            created_at,
                            completed_at,
                        ),
                    )
                    .expect("download task should insert");
            };

        insert_task(
            "old-completed",
            DownloadStatus::Completed,
            "2000-01-01 00:00:00",
            Some("2000-01-01 00:00:00"),
        );
        insert_task(
            "old-failed",
            DownloadStatus::Failed,
            "2000-01-01 00:00:00",
            None,
        );
        insert_task(
            "old-deleted",
            DownloadStatus::Deleted,
            "2000-01-01 00:00:00",
            None,
        );
        insert_task(
            "old-running",
            DownloadStatus::Running,
            "2000-01-01 00:00:00",
            None,
        );
        insert_task(
            "recent-completed",
            DownloadStatus::Completed,
            "2000-01-01 00:00:00",
            Some("2999-01-01 00:00:00"),
        );

        let service = DownloadTaskService::new(&connection, database_path.clone());
        let deleted = service
            .clear_download_records(Some(7))
            .expect("old records should clear");
        assert_eq!(deleted, 3);

        let remaining_ids = service
            .list_created_desc()
            .expect("task list should load")
            .into_iter()
            .map(|task| task.id)
            .collect::<Vec<_>>();
        assert_eq!(remaining_ids.len(), 2);
        assert!(remaining_ids.contains(&"recent-completed".to_string()));
        assert!(remaining_ids.contains(&"old-running".to_string()));

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
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: false,
                aria2_enable_dht6: false,
                aria2_enable_peer_exchange: false,
                aria2_enable_lpd: false,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
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
                http_referrer: None,
                file_conflict_action: None,
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

    #[test]
    fn refresh_all_does_not_resurrect_deleted_task_after_stale_engine_response() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        let (url, request_seen, release_response, server) =
            start_fake_aria2_blocking_status("active");
        save_aria2_settings_with_url(&connection, url);
        insert_aria2_task(
            &connection,
            "stale-refresh-task",
            DownloadStatus::Running,
            "C:\\Downloads",
            "file.bin",
        );
        drop(connection);

        let refresh_database_path = database_path.clone();
        let refresh_thread = thread::spawn(move || {
            let connection =
                db::connect_path(refresh_database_path.clone()).expect("database should migrate");
            DownloadTaskService::new(&connection, refresh_database_path)
                .refresh_all()
                .expect("refresh should not fail");
        });

        request_seen
            .recv_timeout(Duration::from_secs(5))
            .expect("refresh should reach fake aria2");

        let connection = db::connect_path(database_path.clone()).expect("database should reopen");
        DownloadTaskRepository::new(&connection)
            .delete_tasks(&["stale-refresh-task".to_string()])
            .expect("task should delete while refresh is in flight");
        drop(connection);

        release_response
            .send(())
            .expect("fake aria2 response should release");
        refresh_thread.join().expect("refresh thread should finish");
        server.join().expect("fake aria2 should finish");

        let connection = db::connect_path(database_path.clone()).expect("database should reopen");
        let service = DownloadTaskService::new(&connection, database_path.clone());
        assert!(service
            .list_created_desc()
            .expect("task list should load")
            .is_empty());

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn refresh_all_keeps_qbittorrent_running_when_connection_is_unavailable() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        save_qbittorrent_settings_with_url(&connection, "http://127.0.0.1:1".to_string());
        insert_qbittorrent_task(&connection, "qb-refresh-task", DownloadStatus::Running);

        let service = DownloadTaskService::new(&connection, database_path.clone());
        let tasks = service
            .refresh_all()
            .expect("refresh should keep listing tasks");
        let task = tasks
            .iter()
            .find(|task| task.id == "qb-refresh-task")
            .expect("task should remain visible");

        assert_eq!(task.status, DownloadStatus::Running);
        assert_eq!(task.error_message, None);

        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn update_engine_state_can_clear_engine_task_id() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        insert_aria2_task(
            &connection,
            "clear-engine-id-task",
            DownloadStatus::Running,
            "C:\\Downloads",
            "file.bin",
        );
        let repository = DownloadTaskRepository::new(&connection);

        repository
            .update_engine_state(
                "clear-engine-id-task",
                DownloadStatus::Paused,
                12.0,
                0,
                120,
                1_000,
                None,
                None,
            )
            .expect("engine state should update");
        let task = repository
            .get_by_id("clear-engine-id-task")
            .expect("task should load");

        assert_eq!(task.status, DownloadStatus::Paused);
        assert_eq!(task.engine_task_id, None);

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
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: false,
                aria2_enable_dht6: false,
                aria2_enable_peer_exchange: false,
                aria2_enable_lpd: false,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
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

    fn start_fake_aria2_blocking_status(
        status: &'static str,
    ) -> (
        String,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake aria2 should bind");
        let address = listener
            .local_addr()
            .expect("fake aria2 should have address");
        let (request_seen_sender, request_seen_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("fake aria2 request should arrive");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).expect("request should read");
            request_seen_sender
                .send(())
                .expect("request notification should send");
            release_receiver
                .recv()
                .expect("response release should arrive");
            let body = serde_json::json!({
                "jsonrpc": "2.0",
                "id": "unidl",
                "result": {
                    "status": status,
                    "totalLength": "1000",
                    "completedLength": "500",
                    "downloadSpeed": "100",
                }
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("response should write");
        });

        (
            format!("http://{address}/jsonrpc"),
            request_seen_receiver,
            release_sender,
            server,
        )
    }

    fn save_qbittorrent_settings_with_url(connection: &Connection, connection_url: String) {
        EngineSettingsService::new(connection)
            .save(EngineSettingsInput {
                id: "qbittorrent".to_string(),
                engine: EngineKind::QBittorrent,
                name: "qBittorrent".to_string(),
                enabled: true,
                executable_path: None,
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: Some(connection_url),
                username: Some("admin".to_string()),
                password: Some("adminadmin".to_string()),
                remote_path: Some(String::new()),
                supported_source_types: vec![SourceType::Magnet, SourceType::Torrent],
                preferred_domains: Vec::new(),
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: false,
                aria2_enable_dht6: false,
                aria2_enable_peer_exchange: false,
                aria2_enable_lpd: false,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
                priority: 0,
            })
            .expect("qBittorrent settings should save");
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
                tracker_subscription_url: None,
                trackers: Vec::new(),
                proxy_url: None,
                user_agent: None,
                speed_limit_bytes_per_sec: 0,
                qbittorrent_download_limit_bytes_per_sec: 0,
                qbittorrent_upload_limit_bytes_per_sec: 0,
                qbittorrent_seed_ratio_limit: 0.0,
                qbittorrent_seed_time_limit_minutes: 0,
                aria2_enable_dht: false,
                aria2_enable_dht6: false,
                aria2_enable_peer_exchange: false,
                aria2_enable_lpd: false,
                aria2_bt_listen_port: 6881,
                aria2_bt_max_peers: 55,
                aria2_max_connection_per_server: 16,
                aria2_split: 16,
                aria2_min_split_size: "1M".to_string(),
                aria2_file_allocation: "none".to_string(),
                aria2_seed_time: 10,
                aria2_seed_ratio: 1.0,
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

    fn insert_qbittorrent_task(connection: &Connection, id: &str, status: DownloadStatus) {
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
                    SourceType::Magnet.as_db(),
                    "magnet:?xt=urn:btih:abcdef123456",
                    "qbittorrent",
                    EngineKind::QBittorrent.as_db(),
                    "abcdef123456",
                    "file.bin",
                    status.as_db(),
                    "C:\\Downloads",
                ),
            )
            .expect("qBittorrent task should insert");
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
            http_referrer: None,
            created_at: String::new(),
            completed_at: Some(String::new()),
            error_message: None,
            downloaded_file_missing: false,
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

    fn start_fake_tracker_subscriptions(
        bodies: Vec<&'static str>,
    ) -> (Vec<String>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake server should bind");
        let address = listener
            .local_addr()
            .expect("fake server should have address");
        let urls = (0..bodies.len())
            .map(|index| format!("http://{address}/trackers-{index}.txt"))
            .collect();
        let server = thread::spawn(move || {
            for (body, stream) in bodies.into_iter().zip(listener.incoming()) {
                let mut stream = stream.expect("fake server stream should open");
                let mut buffer = [0_u8; 4096];
                let _ = stream.read(&mut buffer).expect("request should read");
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
            }
        });

        (urls, server)
    }

    fn temp_database_path() -> PathBuf {
        std::env::temp_dir().join(format!("unidl-test-{}.sqlite3", Uuid::new_v4()))
    }
}
