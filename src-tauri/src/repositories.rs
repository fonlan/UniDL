use std::error::Error;

use rusqlite::{params, Connection};

use crate::models::{
    AppSettings, AppSettingsInput, DownloadStatus, DownloadTask, EngineKind, EngineSettings,
    EngineSettingsInput, NewDownloadTask, SourceType,
};

pub struct DownloadTaskRepository<'connection> {
    connection: &'connection Connection,
}

pub struct AppSettingsRepository<'connection> {
    connection: &'connection Connection,
}

pub struct EngineSettingsRepository<'connection> {
    connection: &'connection Connection,
}

impl<'connection> AppSettingsRepository<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    pub fn get(&self) -> Result<AppSettings, Box<dyn Error>> {
        Ok(AppSettings {
            web_access_enabled: self.get_value("web_access_enabled")? == "1",
            web_access_password: self.get_value("web_access_password")?,
            web_access_url: self.get_value("web_access_url")?,
        })
    }

    pub fn save(&self, input: &AppSettingsInput) -> Result<AppSettings, Box<dyn Error>> {
        self.save_value(
            "web_access_enabled",
            if input.web_access_enabled { "1" } else { "0" },
        )?;
        self.save_value("web_access_password", &input.web_access_password)?;
        self.save_value("web_access_url", &input.web_access_url)?;
        self.get()
    }

    fn get_value(&self, key: &str) -> Result<String, Box<dyn Error>> {
        let mut statement = self
            .connection
            .prepare("SELECT value FROM app_settings WHERE key = ?1")?;
        let mut rows = statement.query([key])?;

        if let Some(row) = rows.next()? {
            return Ok(row.get("value")?);
        }

        Err(format!("app setting not found: {key}").into())
    }

    fn save_value(&self, key: &str, value: &str) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            INSERT INTO app_settings (key, value, updated_at)
            VALUES (?1, ?2, datetime('now'))
            ON CONFLICT(key) DO UPDATE SET
                value = excluded.value,
                updated_at = datetime('now')
            "#,
            (key, value),
        )?;
        Ok(())
    }
}

impl<'connection> EngineSettingsRepository<'connection> {
    pub fn new(connection: &'connection Connection) -> Self {
        Self { connection }
    }

    pub fn list_all(&self) -> Result<Vec<EngineSettings>, Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                id,
                engine,
                name,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                supported_source_types,
                priority,
                updated_at
            FROM engine_settings
            ORDER BY priority ASC,
            datetime(created_at) ASC,
            id ASC
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
                id,
                engine,
                name,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                supported_source_types,
                priority,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, datetime('now'), datetime('now'))
            ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                enabled = excluded.enabled,
                executable_path = excluded.executable_path,
                default_download_dir = excluded.default_download_dir,
                default_args = excluded.default_args,
                connection_url = excluded.connection_url,
                username = excluded.username,
                password = excluded.password,
                remote_path = excluded.remote_path,
                supported_source_types = excluded.supported_source_types,
                priority = excluded.priority,
                updated_at = datetime('now')
            "#,
            (
                input.id.as_str(),
                input.engine.as_db(),
                input.name.as_str(),
                if input.enabled { 1_i64 } else { 0_i64 },
                input.executable_path.as_deref(),
                input.default_download_dir.as_str(),
                input.default_args.as_str(),
                input.connection_url.as_deref(),
                input.username.as_deref(),
                input.password.as_deref(),
                input.remote_path.as_deref(),
                encode_source_types(&input.supported_source_types),
                input.priority,
            ),
        )?;

        self.get(&input.id)
    }

    pub fn get(&self, id: &str) -> Result<EngineSettings, Box<dyn Error>> {
        let mut statement = self.connection.prepare(
            r#"
            SELECT
                id,
                engine,
                name,
                enabled,
                executable_path,
                default_download_dir,
                default_args,
                connection_url,
                username,
                password,
                remote_path,
                supported_source_types,
                priority,
                updated_at
            FROM engine_settings
            WHERE id = ?1
            "#,
        )?;

        let mut rows = statement.query([id])?;
        if let Some(row) = rows.next()? {
            return read_engine_settings(row);
        }

        Err(format!("engine settings not found: {id}").into())
    }

    pub fn has_download_tasks(&self, id: &str) -> Result<bool, rusqlite::Error> {
        let count: i64 = self.connection.query_row(
            r#"
            SELECT COUNT(*)
            FROM download_tasks
            WHERE engine_settings_id = ?1
                AND status != 'deleted'
            "#,
            [id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn delete(&self, id: &str) -> Result<(), Box<dyn Error>> {
        let deleted = self
            .connection
            .execute("DELETE FROM engine_settings WHERE id = ?1", [id])?;
        if deleted == 0 {
            return Err(format!("engine settings not found: {id}").into());
        }

        Ok(())
    }
}

fn encode_source_types(source_types: &[SourceType]) -> String {
    source_types
        .iter()
        .map(|source_type| source_type.as_db())
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_source_types(value: &str) -> Result<Vec<SourceType>, Box<dyn Error>> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }

    value
        .split(',')
        .map(|source_type| Ok(SourceType::from_db(source_type.trim())?))
        .collect()
}

fn read_engine_settings(row: &rusqlite::Row<'_>) -> Result<EngineSettings, Box<dyn Error>> {
    let engine_value: String = row.get("engine")?;
    let enabled: i64 = row.get("enabled")?;
    let engine = EngineKind::from_db(&engine_value)?;

    Ok(EngineSettings {
        id: row.get("id")?,
        engine,
        name: row.get("name")?,
        enabled: enabled == 1,
        executable_path: row.get("executable_path")?,
        default_download_dir: row.get("default_download_dir")?,
        default_args: row.get("default_args")?,
        connection_url: row.get("connection_url")?,
        username: row.get("username")?,
        password: row.get("password")?,
        remote_path: row.get("remote_path")?,
        supported_source_types: decode_source_types(&row.get::<_, String>("supported_source_types")?)?,
        priority: row.get("priority")?,
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
                engine_settings_id,
                engine,
                engine_task_id,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
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
                engine_settings_id,
                engine,
                engine_task_id,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
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
                engine_settings_id,
                engine,
                engine_task_id,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
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
                engine_settings_id,
                engine,
                engine_task_id,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
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

    pub fn create(&self, id: &str, input: &NewDownloadTask) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            INSERT INTO download_tasks (
                id,
                source_type,
                source,
                engine_settings_id,
                engine,
                file_name,
                status,
                progress,
                speed_bytes_per_sec,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, ?8, ?9, ?10, ?11)
            "#,
            params![
                id,
                input.source_type.as_db(),
                input.source.as_str(),
                input.engine_settings_id.as_str(),
                input.engine.as_db(),
                input.file_name.as_str(),
                DownloadStatus::Queued.as_db(),
                input.save_path.as_str(),
                input.engine_args.as_str(),
                encode_selected_file_indexes(input.selected_file_indexes.as_deref()),
                input.browser_cookies.as_deref(),
            ],
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
            SET status = ?1, error_message = COALESCE(error_message, ?2)
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

fn encode_selected_file_indexes(indexes: Option<&[i64]>) -> Option<String> {
    indexes.map(|values| values.iter().map(ToString::to_string).collect::<Vec<_>>().join(","))
}

fn decode_selected_file_indexes(value: Option<String>) -> Result<Option<Vec<i64>>, Box<dyn Error>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.trim().is_empty() {
        return Ok(Some(Vec::new()));
    }
    Ok(Some(
        value
            .split(",")
            .map(|part| part.trim().parse::<i64>().map_err(Into::into))
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?,
    ))
}

fn read_download_task(row: &rusqlite::Row<'_>) -> Result<DownloadTask, Box<dyn Error>> {
    let source_type: String = row.get("source_type")?;
    let engine: String = row.get("engine")?;
    let status: String = row.get("status")?;

    Ok(DownloadTask {
        id: row.get("id")?,
        source_type: SourceType::from_db(&source_type)?,
        source: row.get("source")?,
        engine_settings_id: row.get("engine_settings_id")?,
        engine: EngineKind::from_db(&engine)?,
        engine_task_id: row.get("engine_task_id")?,
        file_name: row.get("file_name")?,
        status: DownloadStatus::from_db(&status)?,
        progress: row.get("progress")?,
        speed_bytes_per_sec: row.get("speed_bytes_per_sec")?,
        save_path: row.get("save_path")?,
        engine_args: row.get("engine_args")?,
        selected_file_indexes: decode_selected_file_indexes(row.get("selected_file_indexes")?)?,
        browser_cookies: row.get("browser_cookies")?,
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
        error_message: row.get("error_message")?,
    })
}
