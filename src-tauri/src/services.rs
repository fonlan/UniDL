use std::{error::Error, path::PathBuf};

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
        };
        self.repository.create(&id, &task_input)?;
        let task = self.repository.get_by_id(&id)?;

        match engine_adapters::add_task(&engine_settings, &task, self.database_path.clone()) {
            Ok(state) => {
                self.apply_engine_state(&task.id, state)?;
                self.repository.get_by_id(&task.id)
            }
            Err(error) => {
                self.repository.mark_failed(&task.id, &error.to_string())?;
                Err(error)
            }
        }
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
            match engine_adapters::resume_task(&engine_settings, &task) {
                Ok(state) => self.apply_engine_state(&task.id, state)?,
                Err(error) => {
                    self.repository.mark_failed(&task.id, &error.to_string())?;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub fn delete_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        for task in self.repository.list_by_ids(ids)? {
            let engine_settings =
                EngineSettingsRepository::new(self.connection).get(&task.engine_settings_id)?;
            match engine_adapters::delete_task(&engine_settings, &task) {
                Ok(()) => self.repository.delete_tasks(&[task.id])?,
                Err(error) => {
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
            state.engine_task_id.as_deref(),
            state.error_message.as_deref(),
        )?;
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
        Ok(())
    }
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

    pub fn save(&self, input: EngineSettingsInput) -> Result<EngineSettings, Box<dyn Error>> {
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

        let input = EngineSettingsInput {
            name,
            supported_source_types,
            ..input
        };
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
            priority: current.priority,
        })?;

        Ok(EngineInstallResult {
            settings: next,
            version: installed.version,
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
                default_args: "--continue=true".to_string(),
                connection_url: Some("http://127.0.0.1:6800/jsonrpc".to_string()),
                username: None,
                password: None,
                remote_path: None,
                supported_source_types: vec![SourceType::Http, SourceType::Ftp],
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
            .delete_tasks(std::slice::from_ref(&task.id))
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

    fn temp_database_path() -> PathBuf {
        std::env::temp_dir().join(format!("unidl-test-{}.sqlite3", Uuid::new_v4()))
    }
}
