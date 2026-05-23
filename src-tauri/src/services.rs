use std::{error::Error, path::PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

use crate::{
    engine_adapters,
    models::{
        engine_supports_source_type, AppSettings, AppSettingsInput, CreateDownloadTaskInput,
        DownloadStatus, DownloadTask, EngineKind, EngineSettings, EngineSettingsInput, SourceType,
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

        let engine_settings = EngineSettingsRepository::new(self.connection).get(input.engine)?;
        if !engine_settings.enabled {
            return Err(format!("{} is disabled", input.engine.as_db()).into());
        }
        if !engine_supports_source_type(input.engine, input.source_type) {
            return Err(format!(
                "{} does not support {} tasks",
                input.engine.as_db(),
                input.source_type.as_db()
            )
            .into());
        }

        let id = Uuid::new_v4().to_string();
        self.repository.create(&id, &input)?;
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
                EngineSettingsRepository::new(self.connection).get(task.engine)?;
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
                EngineSettingsRepository::new(self.connection).get(task.engine)?;
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
                EngineSettingsRepository::new(self.connection).get(task.engine)?;
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
        let engine_settings = EngineSettingsRepository::new(self.connection).get(task.engine)?;
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
        self.repository.save(&input)
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
    fn qbittorrent_task_lifecycle_add_pause_resume_delete() {
        let (base_url, hits, server) = start_fake_qbittorrent(8);
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");

        EngineSettingsService::new(&connection)
            .save(EngineSettingsInput {
                engine: EngineKind::QBittorrent,
                enabled: true,
                executable_path: None,
                default_download_dir: String::new(),
                default_args: String::new(),
                connection_url: Some(base_url),
                username: Some("admin".to_string()),
                password: Some("adminadmin".to_string()),
                remote_path: Some(String::new()),
            })
            .expect("qBittorrent settings should save");

        let service = DownloadTaskService::new(&connection, database_path.clone());
        let task = service
            .create(CreateDownloadTaskInput {
                source_type: SourceType::Magnet,
                source: "magnet:?xt=urn:btih:ABCDEF123456&dn=unidl".to_string(),
                engine: EngineKind::QBittorrent,
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
