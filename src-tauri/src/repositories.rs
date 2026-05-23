use std::error::Error;

use rusqlite::Connection;

use crate::models::{
    supported_source_types, CreateDownloadTaskInput, DownloadStatus, DownloadTask, EngineKind,
    EngineSettings, EngineSettingsInput, SourceType,
};

pub struct DownloadTaskRepository<'connection> {
    connection: &'connection Connection,
}

pub struct EngineSettingsRepository<'connection> {
    connection: &'connection Connection,
}

impl<'connection> EngineSettingsRepository<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    pub fn list_all(&self) -> Result<Vec<EngineSettings>, Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                engine,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                updated_at
            FROM engine_settings
            ORDER BY CASE engine
                WHEN 'aria2' THEN 1
                WHEN 'yt-dlp' THEN 2
                WHEN 'qbittorrent' THEN 3
                ELSE 4
            END
            "#,
        )?;

        let mut rows = statement.query([])?;
        let mut settings = Vec::new();

        while let Some(row) = rows.next()? {
            settings.push(read_engine_settings(row)?);
        }

        Ok(settings)
    }

    pub fn save(&self, input: &EngineSettingsInput) -> Result<EngineSettings, Box<dyn Error>> {
        self.connection.execute(
            r#"
            INSERT INTO engine_settings (
                engine,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))
            ON CONFLICT(engine) DO UPDATE SET
                enabled = excluded.enabled,
                executable_path = excluded.executable_path,
                default_download_dir = excluded.default_download_dir,
                default_args = excluded.default_args,
                connection_url = excluded.connection_url,
                username = excluded.username,
                password = excluded.password,
                remote_path = excluded.remote_path,
                updated_at = datetime('now')
            "#,
            (
                input.engine.as_db(),
                if input.enabled { 1_i64 } else { 0_i64 },
                input.executable_path.as_deref(),
                input.default_download_dir.as_str(),
                input.default_args.as_str(),
                input.connection_url.as_deref(),
                input.username.as_deref(),
                input.password.as_deref(),
                input.remote_path.as_deref(),
            ),
        )?;

        self.get(input.engine)
    }

    pub fn get(&self, engine: EngineKind) -> Result<EngineSettings, Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                engine,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                updated_at
            FROM engine_settings
            WHERE engine = ?1
            "#,
        )?;

        let mut rows = statement.query([engine.as_db()])?;
        if let Some(row) = rows.next()? {
            return read_engine_settings(row);
        }

        Err(format!("engine settings not found: {}", engine.as_db()).into())
    }
}

fn read_engine_settings(row: &rusqlite::Row<'_>) -> Result<EngineSettings, Box<dyn Error>> {
    let engine_value: String = row.get("engine")?;
    let enabled: i64 = row.get("enabled")?;
    let engine = EngineKind::from_db(&engine_value)?;

    Ok(EngineSettings {
        engine,
        enabled: enabled == 1,
        executable_path: row.get("executable_path")?,
        default_download_dir: row.get("default_download_dir")?,
        default_args: row.get("default_args")?,
        connection_url: row.get("connection_url")?,
        username: row.get("username")?,
        password: row.get("password")?,
        remote_path: row.get("remote_path")?,
        supported_source_types: supported_source_types(engine),
        updated_at: row.get("updated_at")?,
    })
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
                engine_args,
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
            tasks.push(read_download_task(row)?);
        }

        Ok(tasks)
    }

    pub fn list_by_ids(&self, ids: &[String]) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
        let mut tasks = Vec::new();
        for id in ids {
            tasks.push(self.get_by_id(id)?);
        }
        Ok(tasks)
    }

    pub fn list_unfinished(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
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
                engine_args,
                created_at,
                completed_at,
                error_message
            FROM download_tasks
            WHERE status IN ('queued', 'running')
            ORDER BY datetime(created_at) DESC, created_at DESC
            "#,
        )?;

        let mut rows = statement.query([])?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next()? {
            tasks.push(read_download_task(row)?);
        }
        Ok(tasks)
    }

    pub fn list_paused(&self) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
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
                engine_args,
                created_at,
                completed_at,
                error_message
            FROM download_tasks
            WHERE status = 'paused'
            ORDER BY datetime(created_at) DESC, created_at DESC
            "#,
        )?;

        let mut rows = statement.query([])?;
        let mut tasks = Vec::new();
        while let Some(row) = rows.next()? {
            tasks.push(read_download_task(row)?);
        }
        Ok(tasks)
    }

    pub fn get_by_id(&self, id: &str) -> Result<DownloadTask, Box<dyn Error>> {
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
                engine_args,
                created_at,
                completed_at,
                error_message
            FROM download_tasks
            WHERE id = ?1
            "#,
        )?;

        let mut rows = statement.query([id])?;
        if let Some(row) = rows.next()? {
            return read_download_task(row);
        }

        Err(format!("download task not found: {}", id).into())
    }

    pub fn create(&self, id: &str, input: &CreateDownloadTaskInput) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            INSERT INTO download_tasks (
                id,
                source_type,
                source,
                engine,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, 0, ?7, ?8)
            "#,
            (
                id,
                input.source_type.as_db(),
                input.source.as_str(),
                input.engine.as_db(),
                input.file_name.as_str(),
                DownloadStatus::Queued.as_db(),
                input.save_path.as_str(),
                input.engine_args.as_str(),
            ),
        )?;
        Ok(())
    }

    pub fn update_engine_state(
        &self,
        id: &str,
        status: DownloadStatus,
        progress: f64,
        speed_bytes_per_sec: i64,
        engine_task_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<(), rusqlite::Error> {
        let completed_at_sql = if status == DownloadStatus::Completed {
            "datetime('now')"
        } else {
            "completed_at"
        };
        let sql = format!(
            r#"
            UPDATE download_tasks
            SET
                status = ?1,
                progress = ?2,
                speed_bytes_per_sec = ?3,
                engine_task_id = COALESCE(?4, engine_task_id),
                error_message = ?5,
                completed_at = {}
            WHERE id = ?6
            "#,
            completed_at_sql
        );

        self.connection.execute(
            &sql,
            (
                status.as_db(),
                progress,
                speed_bytes_per_sec,
                engine_task_id,
                error_message,
                id,
            ),
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, id: &str, error: &str) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            UPDATE download_tasks
            SET status = ?1, error_message = ?2
            WHERE id = ?3
            "#,
            (DownloadStatus::Failed.as_db(), error, id),
        )?;
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
}

fn read_download_task(row: &rusqlite::Row<'_>) -> Result<DownloadTask, Box<dyn Error>> {
    let source_type: String = row.get("source_type")?;
    let engine: String = row.get("engine")?;
    let status: String = row.get("status")?;

    Ok(DownloadTask {
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
        engine_args: row.get("engine_args")?,
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
        error_message: row.get("error_message")?,
    })
}
