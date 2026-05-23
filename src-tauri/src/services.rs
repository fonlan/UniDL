use std::error::Error;

use rusqlite::Connection;
use uuid::Uuid;

use crate::{
    models::{
        engine_supports_source_type, CreateDownloadTaskInput, DownloadTask, EngineKind,
        EngineSettings, EngineSettingsInput, SourceType,
    },
    repositories::{DownloadTaskRepository, EngineSettingsRepository},
};

pub struct DownloadTaskService<'connection> {
    connection: &'connection Connection,
    repository: DownloadTaskRepository<'connection>,
}

impl<'connection> DownloadTaskService<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self {
            connection,
            repository: DownloadTaskRepository::new(connection),
        }
    }

    pub fn list_created_desc(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        self.repository.list_created_desc()
    }

    pub fn create(
        &self,
        input: CreateDownloadTaskInput,
    ) -> Result<DownloadTask, Box<dyn Error>> {
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
        self.repository.get_by_id(&id)
    }

    pub fn pause_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        self.repository.pause_tasks(ids)?;
        Ok(())
    }

    pub fn resume_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        self.repository.resume_tasks(ids)?;
        Ok(())
    }

    pub fn delete_tasks(&self, ids: &[String]) -> Result<(), Box<dyn Error>> {
        self.repository.delete_tasks(ids)?;
        Ok(())
    }

    pub fn pause_all_unfinished(&self) -> Result<(), Box<dyn Error>> {
        self.repository.pause_all_unfinished()?;
        Ok(())
    }

    pub fn resume_all_paused(&self) -> Result<(), Box<dyn Error>> {
        self.repository.resume_all_paused()?;
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
