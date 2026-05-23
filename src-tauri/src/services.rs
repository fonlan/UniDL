use std::{error::Error, path::PathBuf};

use rusqlite::Connection;
use uuid::Uuid;

use crate::{
    engine_adapters,
    models::{
        engine_supports_source_type, CreateDownloadTaskInput, DownloadStatus, DownloadTask,
        EngineKind, EngineSettings, EngineSettingsInput, SourceType,
    },
    repositories::{DownloadTaskRepository, EngineSettingsRepository},
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
