use std::{error::Error, fs, path::PathBuf};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

pub fn database_path(app: &AppHandle) -> Result<PathBuf, Box<dyn Error>> {
    let data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&data_dir)?;
    Ok(data_dir.join("unidl.sqlite3"))
}

pub fn connect_path(path: PathBuf) -> Result<Connection, Box<dyn Error>> {
    let connection = Connection::open(path)?;
    migrate(&connection)?;
    Ok(connection)
}

fn migrate(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS engine_settings (
            id TEXT PRIMARY KEY,
            engine TEXT NOT NULL CHECK (engine IN ('aria2', 'yt-dlp', 'qbittorrent')),
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            executable_path TEXT,
            default_download_dir TEXT NOT NULL DEFAULT '',
            default_args TEXT NOT NULL DEFAULT '',
            connection_url TEXT,
            username TEXT,
            password TEXT,
            remote_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS app_settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS download_tasks (
            id TEXT PRIMARY KEY,
            source_type TEXT NOT NULL CHECK (
                source_type IN ('http', 'ftp', 'magnet', 'torrent')
            ),
            source TEXT NOT NULL,
            engine_settings_id TEXT NOT NULL,
            engine TEXT NOT NULL CHECK (engine IN ('aria2', 'yt-dlp', 'qbittorrent')),
            engine_task_id TEXT,
            file_name TEXT NOT NULL,
            status TEXT NOT NULL CHECK (
                status IN ('queued', 'running', 'paused', 'completed', 'failed', 'deleted')
            ),
            progress REAL NOT NULL DEFAULT 0 CHECK (progress >= 0 AND progress <= 100),
            speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (speed_bytes_per_sec >= 0),
            save_path TEXT NOT NULL,
            engine_args TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            completed_at TEXT,
            error_message TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_download_tasks_created_at
            ON download_tasks (created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_download_tasks_status
            ON download_tasks (status);
        "#,
    )?;

    migrate_engine_settings_default_args(connection)?;
    migrate_engine_settings_ids(connection)?;
    migrate_download_tasks_engine_settings_id(connection)?;
    migrate_download_tasks_engine_args(connection)?;
    seed_engine_settings(connection)?;
    seed_app_settings(connection)
}

fn has_column(
    connection: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table_name})"))?;
    let columns = statement.query_map([], |row| row.get::<_, String>("name"))?;

    for column in columns {
        if column? == column_name {
            return Ok(true);
        }
    }

    Ok(false)
}

fn migrate_engine_settings_ids(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "id")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE engine_settings RENAME TO engine_settings_legacy;

        CREATE TABLE engine_settings (
            id TEXT PRIMARY KEY,
            engine TEXT NOT NULL CHECK (engine IN ('aria2', 'yt-dlp', 'qbittorrent')),
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            executable_path TEXT,
            default_download_dir TEXT NOT NULL DEFAULT '',
            default_args TEXT NOT NULL DEFAULT '',
            connection_url TEXT,
            username TEXT,
            password TEXT,
            remote_path TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        INSERT INTO engine_settings (
            id,
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
        )
        SELECT
            engine,
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
        FROM engine_settings_legacy;

        DROP TABLE engine_settings_legacy;
        "#,
    )
}

fn migrate_engine_settings_default_args(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "default_args")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN default_args TEXT NOT NULL DEFAULT '';
        "#,
    )
}

fn migrate_download_tasks_engine_settings_id(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if has_column(connection, "download_tasks", "engine_settings_id")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE download_tasks
            ADD COLUMN engine_settings_id TEXT NOT NULL DEFAULT '';

        UPDATE download_tasks
        SET engine_settings_id = engine
        WHERE engine_settings_id = '';
        "#,
    )
}

fn migrate_download_tasks_engine_args(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "download_tasks", "engine_args")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE download_tasks
            ADD COLUMN engine_args TEXT NOT NULL DEFAULT '';
        "#,
    )
}

fn seed_engine_settings(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO engine_settings (
            id,
            engine,
            enabled,
            executable_path,
            default_download_dir,
            default_args,
            connection_url,
            remote_path
        ) VALUES
            ('aria2', 'aria2', 0, '%AppData%\UniDL\engines\aria2c.exe', '', '--continue=true', 'http://127.0.0.1:6800/jsonrpc', NULL),
            ('yt-dlp', 'yt-dlp', 0, '%AppData%\UniDL\engines\yt-dlp.exe', '', '--newline', NULL, NULL),
            ('qbittorrent', 'qbittorrent', 0, NULL, '', '', 'http://127.0.0.1:8080', '');
        "#,
    )
}

fn seed_app_settings(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO app_settings (key, value) VALUES
            ('web_access_enabled', '0'),
            ('web_access_password', '');
        "#,
    )
}
