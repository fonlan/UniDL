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
            private_download_domains: decode_domains(&self.get_value("private_download_domains")?),
            app_proxy_url: self
                .get_optional_value("app_proxy_url")?
                .unwrap_or_default(),
            auto_start_enabled: self.get_value("auto_start_enabled")? == "1",
            auto_start_minimized_to_tray: self.get_value("auto_start_minimized_to_tray")? == "1",
            close_to_tray_enabled: self.get_value("close_to_tray_enabled")? == "1",
            download_completion_notification_enabled: self
                .get_value("download_completion_notification_enabled")?
                == "1",
            prevent_sleep_when_downloading_enabled: self
                .get_value("prevent_sleep_when_downloading_enabled")?
                == "1",
            prevent_sleep_when_web_access_enabled: self
                .get_value("prevent_sleep_when_web_access_enabled")?
                == "1",
            local_download_concurrency: self.get_value("local_download_concurrency")?.parse()?,
            auto_clean_download_tasks_enabled: self
                .get_value("auto_clean_download_tasks_enabled")?
                == "1",
            auto_clean_download_tasks_days: self
                .get_value("auto_clean_download_tasks_days")?
                .parse()?,
        })
    }

    pub fn save(&self, input: &AppSettingsInput) -> Result<AppSettings, Box<dyn Error>> {
        self.save_value(
            "web_access_enabled",
            if input.web_access_enabled { "1" } else { "0" },
        )?;
        self.save_value("web_access_password", &input.web_access_password)?;
        self.save_value("web_access_url", &input.web_access_url)?;
        self.save_value(
            "private_download_domains",
            &encode_domains(&input.private_download_domains),
        )?;
        self.save_value("app_proxy_url", input.app_proxy_url.trim())?;
        self.save_value(
            "auto_start_enabled",
            if input.auto_start_enabled { "1" } else { "0" },
        )?;
        self.save_value(
            "auto_start_minimized_to_tray",
            if input.auto_start_minimized_to_tray {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "close_to_tray_enabled",
            if input.close_to_tray_enabled {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "download_completion_notification_enabled",
            if input.download_completion_notification_enabled {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "prevent_sleep_when_downloading_enabled",
            if input.prevent_sleep_when_downloading_enabled {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "prevent_sleep_when_web_access_enabled",
            if input.prevent_sleep_when_web_access_enabled {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "local_download_concurrency",
            &input.local_download_concurrency.to_string(),
        )?;
        self.save_value(
            "auto_clean_download_tasks_enabled",
            if input.auto_clean_download_tasks_enabled {
                "1"
            } else {
                "0"
            },
        )?;
        self.save_value(
            "auto_clean_download_tasks_days",
            &input.auto_clean_download_tasks_days.to_string(),
        )?;
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

    fn get_optional_value(&self, key: &str) -> Result<Option<String>, Box<dyn Error>> {
        let mut statement = self
            .connection
            .prepare("SELECT value FROM app_settings WHERE key = ?1")?;
        let mut rows = statement.query([key])?;

        if let Some(row) = rows.next()? {
            return Ok(Some(row.get("value")?));
        }

        Ok(None)
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

    pub fn connection(&self) -> &'connection Connection {
        self.connection
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
                preferred_domains,
                tracker_subscription_url,
                trackers,
                proxy_url,
                user_agent,
                speed_limit_bytes_per_sec,
                qbittorrent_download_limit_bytes_per_sec,
                qbittorrent_upload_limit_bytes_per_sec,
                qbittorrent_seed_ratio_limit,
                qbittorrent_seed_time_limit_minutes,
                aria2_enable_dht,
                aria2_enable_dht6,
                aria2_enable_peer_exchange,
                aria2_enable_lpd,
                aria2_bt_listen_port,
                aria2_bt_max_peers,
                aria2_max_connection_per_server,
                aria2_split,
                aria2_min_split_size,
                aria2_file_allocation,
                aria2_seed_time,
                aria2_seed_ratio,
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
                preferred_domains,
                tracker_subscription_url,
                trackers,
                proxy_url,
                user_agent,
                speed_limit_bytes_per_sec,
                qbittorrent_download_limit_bytes_per_sec,
                qbittorrent_upload_limit_bytes_per_sec,
                qbittorrent_seed_ratio_limit,
                qbittorrent_seed_time_limit_minutes,
                aria2_enable_dht,
                aria2_enable_dht6,
                aria2_enable_peer_exchange,
                aria2_enable_lpd,
                aria2_bt_listen_port,
                aria2_bt_max_peers,
                aria2_max_connection_per_server,
                aria2_split,
                aria2_min_split_size,
                aria2_file_allocation,
                aria2_seed_time,
                aria2_seed_ratio,
                priority,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32, ?33, ?34, ?35, datetime('now'), datetime('now'))
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
                preferred_domains = excluded.preferred_domains,
                tracker_subscription_url = excluded.tracker_subscription_url,
                trackers = excluded.trackers,
                proxy_url = excluded.proxy_url,
                user_agent = excluded.user_agent,
                speed_limit_bytes_per_sec = excluded.speed_limit_bytes_per_sec,
                qbittorrent_download_limit_bytes_per_sec = excluded.qbittorrent_download_limit_bytes_per_sec,
                qbittorrent_upload_limit_bytes_per_sec = excluded.qbittorrent_upload_limit_bytes_per_sec,
                qbittorrent_seed_ratio_limit = excluded.qbittorrent_seed_ratio_limit,
                qbittorrent_seed_time_limit_minutes = excluded.qbittorrent_seed_time_limit_minutes,
                aria2_enable_dht = excluded.aria2_enable_dht,
                aria2_enable_dht6 = excluded.aria2_enable_dht6,
                aria2_enable_peer_exchange = excluded.aria2_enable_peer_exchange,
                aria2_enable_lpd = excluded.aria2_enable_lpd,
                aria2_bt_listen_port = excluded.aria2_bt_listen_port,
                aria2_bt_max_peers = excluded.aria2_bt_max_peers,
                aria2_max_connection_per_server = excluded.aria2_max_connection_per_server,
                aria2_split = excluded.aria2_split,
                aria2_min_split_size = excluded.aria2_min_split_size,
                aria2_file_allocation = excluded.aria2_file_allocation,
                aria2_seed_time = excluded.aria2_seed_time,
                aria2_seed_ratio = excluded.aria2_seed_ratio,
                priority = excluded.priority,
                updated_at = datetime('now')
            "#,
            params![
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
                encode_domains(&input.preferred_domains),
                input.tracker_subscription_url.as_deref(),
                encode_domains(&input.trackers),
                input.proxy_url.as_deref(),
                input.user_agent.as_deref(),
                input.speed_limit_bytes_per_sec,
                input.qbittorrent_download_limit_bytes_per_sec,
                input.qbittorrent_upload_limit_bytes_per_sec,
                input.qbittorrent_seed_ratio_limit,
                input.qbittorrent_seed_time_limit_minutes,
                if input.aria2_enable_dht { 1_i64 } else { 0_i64 },
                if input.aria2_enable_dht6 { 1_i64 } else { 0_i64 },
                if input.aria2_enable_peer_exchange { 1_i64 } else { 0_i64 },
                if input.aria2_enable_lpd { 1_i64 } else { 0_i64 },
                input.aria2_bt_listen_port,
                input.aria2_bt_max_peers,
                input.aria2_max_connection_per_server,
                input.aria2_split,
                input.aria2_min_split_size.as_str(),
                input.aria2_file_allocation.as_str(),
                input.aria2_seed_time,
                input.aria2_seed_ratio,
                input.priority,
            ],
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
                preferred_domains,
                tracker_subscription_url,
                trackers,
                proxy_url,
                user_agent,
                speed_limit_bytes_per_sec,
                qbittorrent_download_limit_bytes_per_sec,
                qbittorrent_upload_limit_bytes_per_sec,
                qbittorrent_seed_ratio_limit,
                qbittorrent_seed_time_limit_minutes,
                aria2_enable_dht,
                aria2_enable_dht6,
                aria2_enable_peer_exchange,
                aria2_enable_lpd,
                aria2_bt_listen_port,
                aria2_bt_max_peers,
                aria2_max_connection_per_server,
                aria2_split,
                aria2_min_split_size,
                aria2_file_allocation,
                aria2_seed_time,
                aria2_seed_ratio,
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

fn encode_domains(domains: &[String]) -> String {
    domains
        .iter()
        .map(|domain| domain.trim().to_lowercase())
        .filter(|domain| !domain.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

fn decode_domains(value: &str) -> Vec<String> {
    value
        .split(",")
        .map(|domain| domain.trim().to_string())
        .filter(|domain| !domain.is_empty())
        .collect()
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
    let aria2_enable_dht: i64 = row.get("aria2_enable_dht")?;
    let aria2_enable_dht6: i64 = row.get("aria2_enable_dht6")?;
    let aria2_enable_peer_exchange: i64 = row.get("aria2_enable_peer_exchange")?;
    let aria2_enable_lpd: i64 = row.get("aria2_enable_lpd")?;
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
        supported_source_types: decode_source_types(
            &row.get::<_, String>("supported_source_types")?,
        )?,
        preferred_domains: decode_domains(&row.get::<_, String>("preferred_domains")?),
        tracker_subscription_url: row.get("tracker_subscription_url")?,
        trackers: decode_domains(&row.get::<_, String>("trackers")?),
        proxy_url: row.get("proxy_url")?,
        user_agent: row.get("user_agent")?,
        speed_limit_bytes_per_sec: row.get("speed_limit_bytes_per_sec")?,
        qbittorrent_download_limit_bytes_per_sec: row
            .get("qbittorrent_download_limit_bytes_per_sec")?,
        qbittorrent_upload_limit_bytes_per_sec: row
            .get("qbittorrent_upload_limit_bytes_per_sec")?,
        qbittorrent_seed_ratio_limit: row.get("qbittorrent_seed_ratio_limit")?,
        qbittorrent_seed_time_limit_minutes: row.get("qbittorrent_seed_time_limit_minutes")?,
        aria2_enable_dht: aria2_enable_dht == 1,
        aria2_enable_dht6: aria2_enable_dht6 == 1,
        aria2_enable_peer_exchange: aria2_enable_peer_exchange == 1,
        aria2_enable_lpd: aria2_enable_lpd == 1,
        aria2_bt_listen_port: row.get("aria2_bt_listen_port")?,
        aria2_bt_max_peers: row.get("aria2_bt_max_peers")?,
        aria2_max_connection_per_server: row.get("aria2_max_connection_per_server")?,
        aria2_split: row.get("aria2_split")?,
        aria2_min_split_size: row.get("aria2_min_split_size")?,
        aria2_file_allocation: row.get("aria2_file_allocation")?,
        aria2_seed_time: row.get("aria2_seed_time")?,
        aria2_seed_ratio: row.get("aria2_seed_ratio")?,
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer,
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer,
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

    pub fn has_active_downloads(&self) -> Result<bool, rusqlite::Error> {
        let count: i64 = self.connection.query_row(
            r#"
            SELECT COUNT(*)
            FROM download_tasks
            WHERE status IN ('queued', 'running')
            "#,
            [],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn count_running_local_downloads(&self) -> Result<i64, rusqlite::Error> {
        self.connection.query_row(
            r#"
            SELECT COUNT(*)
            FROM download_tasks
            WHERE status = 'running'
              AND engine IN ('aria2', 'yt-dlp')
            "#,
            [],
            |row| row.get(0),
        )
    }

    pub fn list_queued_local_oldest(
        &self,
        limit: i64,
    ) -> Result<Vec<DownloadTask>, Box<dyn Error>> {
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer,
                created_at,
                completed_at,
                error_message
            FROM download_tasks
            WHERE status = 'queued'
              AND engine IN ('aria2', 'yt-dlp')
            ORDER BY datetime(created_at) ASC, created_at ASC
            LIMIT ?1
            "#,
        )?;

        let mut rows = statement.query([limit])?;
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer,
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer,
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
                downloaded_bytes,
                total_bytes,
                save_path,
                engine_args,
                selected_file_indexes,
                browser_cookies,
                http_referrer
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, 0, ?8, ?9, ?10, ?11, ?12)
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
                input.http_referrer.as_deref(),
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
        downloaded_bytes: i64,
        total_bytes: i64,
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
                downloaded_bytes = ?4,
                total_bytes = ?5,
                engine_task_id = ?6,
                error_message = ?7,
                completed_at = {}
            WHERE id = ?8
            "#,
            completed_at_sql
        );

        self.connection.execute(
            &sql,
            (
                status.as_db(),
                progress,
                speed_bytes_per_sec,
                downloaded_bytes,
                total_bytes,
                engine_task_id,
                error_message,
                id,
            ),
        )?;
        Ok(())
    }

    pub fn update_engine_state_if_current(
        &self,
        id: &str,
        expected_status: DownloadStatus,
        expected_engine_task_id: Option<&str>,
        status: DownloadStatus,
        progress: f64,
        speed_bytes_per_sec: i64,
        downloaded_bytes: i64,
        total_bytes: i64,
        engine_task_id: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<bool, rusqlite::Error> {
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
                downloaded_bytes = ?4,
                total_bytes = ?5,
                engine_task_id = ?6,
                error_message = ?7,
                completed_at = {}
            WHERE id = ?8
              AND status = ?9
              AND (
                (?10 IS NULL AND engine_task_id IS NULL)
                OR engine_task_id = ?10
              )
            "#,
            completed_at_sql
        );

        let changed = self.connection.execute(
            &sql,
            params![
                status.as_db(),
                progress,
                speed_bytes_per_sec,
                downloaded_bytes,
                total_bytes,
                engine_task_id,
                error_message,
                id,
                expected_status.as_db(),
                expected_engine_task_id,
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn update_selected_file_indexes(
        &self,
        id: &str,
        indexes: Option<&[i64]>,
    ) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            UPDATE download_tasks
            SET selected_file_indexes = ?1
            WHERE id = ?2
            "#,
            (encode_selected_file_indexes(indexes), id),
        )?;
        Ok(())
    }

    pub fn update_file_name(&self, id: &str, file_name: &str) -> Result<(), rusqlite::Error> {
        self.connection.execute(
            r#"
            UPDATE download_tasks
            SET file_name = ?1
            WHERE id = ?2
            "#,
            (file_name, id),
        )?;
        Ok(())
    }

    pub fn mark_failed_if_current(
        &self,
        id: &str,
        expected_status: DownloadStatus,
        expected_engine_task_id: Option<&str>,
        error: &str,
    ) -> Result<bool, rusqlite::Error> {
        let changed = self.connection.execute(
            r#"
            UPDATE download_tasks
            SET status = ?1, error_message = COALESCE(error_message, ?2)
            WHERE id = ?3
              AND status = ?4
              AND (
                (?5 IS NULL AND engine_task_id IS NULL)
                OR engine_task_id = ?5
              )
            "#,
            params![
                DownloadStatus::Failed.as_db(),
                error,
                id,
                expected_status.as_db(),
                expected_engine_task_id,
            ],
        )?;
        Ok(changed > 0)
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

    pub fn clear_download_records(
        &self,
        older_than_days: Option<i64>,
    ) -> Result<usize, rusqlite::Error> {
        match older_than_days {
            Some(days) => {
                let cutoff = format!("-{days} days");
                self.connection.execute(
                    r#"
                    DELETE FROM download_tasks
                    WHERE status IN (?1, ?2, ?3)
                      AND datetime(COALESCE(completed_at, created_at)) < datetime('now', ?4)
                    "#,
                    params![
                        DownloadStatus::Completed.as_db(),
                        DownloadStatus::Failed.as_db(),
                        DownloadStatus::Deleted.as_db(),
                        cutoff,
                    ],
                )
            }
            None => self.connection.execute(
                r#"
                DELETE FROM download_tasks
                WHERE status IN (?1, ?2, ?3)
                "#,
                params![
                    DownloadStatus::Completed.as_db(),
                    DownloadStatus::Failed.as_db(),
                    DownloadStatus::Deleted.as_db(),
                ],
            ),
        }
    }
}

fn encode_selected_file_indexes(indexes: Option<&[i64]>) -> Option<String> {
    indexes.map(|values| {
        values
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    })
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
        downloaded_bytes: row.get("downloaded_bytes")?,
        total_bytes: row.get("total_bytes")?,
        save_path: row.get("save_path")?,
        engine_args: row.get("engine_args")?,
        selected_file_indexes: decode_selected_file_indexes(row.get("selected_file_indexes")?)?,
        browser_cookies: row.get("browser_cookies")?,
        http_referrer: row.get("http_referrer")?,
        created_at: row.get("created_at")?,
        completed_at: row.get("completed_at")?,
        error_message: row.get("error_message")?,
        downloaded_file_missing: false,
    })
}
