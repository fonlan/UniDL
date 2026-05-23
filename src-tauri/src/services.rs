use std::error::Error;

use rusqlite::Connection;

use crate::{models::DownloadTask, repositories::DownloadTaskRepository};

pub struct DownloadTaskService<'connection> {
    repository: DownloadTaskRepository<'connection>,
}

impl<'connection> DownloadTaskService<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self {
            repository: DownloadTaskRepository::new(connection),
        }
    }

    pub fn list_created_desc(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        self.repository.list_created_desc()
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
