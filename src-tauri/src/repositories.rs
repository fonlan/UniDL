use std::error::Error;

use rusqlite::Connection;

use crate::models::{DownloadStatus, DownloadTask, EngineKind, SourceType};

pub struct DownloadTaskRepository<'connection> {
    connection: &'connection Connection,
}

impl<'connection> DownloadTaskRepository<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    pub fn list_created_desc(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                id,
                source_type,
                source,
                engine,
                engine_task_id,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                created_at,
                completed_at,
                error_message
            FROM download_tasks
            WHERE status != 'deleted'
            ORDER BY datetime(created_at) DESC, created_at DESC
            "#,
        )?;

        let mut rows = statement.query([])?;
        let mut tasks = Vec::new();

        while let Some(row) = rows.next()? {
            let source_type: String = row.get("source_type")?;
            let engine: String = row.get("engine")?;
            let status: String = row.get("status")?;

            tasks.push(DownloadTask {
                id: row.get("id")?,
                source_type: SourceType::from_db(&source_type)?,
                source: row.get("source")?,
                engine: EngineKind::from_db(&engine)?,
                engine_task_id: row.get("engine_task_id")?,
                file_name: row.get("file_name")?,
                status: DownloadStatus::from_db(&status)?,
                progress: row.get("progress")?,
                speed_bytes_per_sec: row.get("speed_bytes_per_sec")?,
                save_path: row.get("save_path")?,
                created_at: row.get("created_at")?,
                completed_at: row.get("completed_at")?,
                error_message: row.get("error_message")?,
            });
        }

        Ok(tasks)
    }

    pub fn pause_tasks(&self, ids: &[String]) -> Result<(), rusqlite::Error> {
        for id in ids {
            self.connection.execute(
                r#"
                UPDATE download_tasks
                SET status = ?1
                WHERE id = ?2 AND status IN ('queued', 'running')
                "#,
                (DownloadStatus::Paused.as_db(), id),
            )?;
        }
        Ok(())
    }

    pub fn resume_tasks(&self, ids: &[String]) -> Result<(), rusqlite::Error> {
        for id in ids {
            self.connection.execute(
                r#"
                UPDATE download_tasks
                SET status = ?1
                WHERE id = ?2 AND status = 'paused'
                "#,
                (DownloadStatus::Queued.as_db(), id),
            )?;
        }
        Ok(())
    }

    pub fn delete_tasks(&self, ids: &[String]) -> Result<(), rusqlite::Error> {
        for id in ids {
            self.connection.execute(
                r#"
                UPDATE download_tasks
                SET status = ?1
                WHERE id = ?2
                "#,
                (DownloadStatus::Deleted.as_db(), id),
            )?;
        }
        Ok(())
    }

    pub fn pause_all_unfinished(&self) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            UPDATE download_tasks
            SET status = ?1
            WHERE status IN ('queued', 'running')
            "#,
            [DownloadStatus::Paused.as_db()],
        )?;
        Ok(())
    }

    pub fn resume_all_paused(&self) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            UPDATE download_tasks
            SET status = ?1
            WHERE status = 'paused'
            "#,
            [DownloadStatus::Queued.as_db()],
        )?;
        Ok(())
    }
}
