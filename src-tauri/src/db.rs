use std::{error::Error, fs};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

pub fn connect(app: &AppHandle) -> Result<Connection, Box<dyn Error>> {
    let data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&data_dir)?;

    let connection = Connection::open(data_dir.join("unidl.sqlite3"))?;
    migrate(&connection)?;
    Ok(connection)
}

fn migrate(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS engine_settings (
            engine TEXT PRIMARY KEY CHECK (engine IN ('aria2', 'yt-dlp', 'qbittorrent')),
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            executable_path TEXT,
            default_download_dir TEXT NOT NULL DEFAULT '',
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
            engine TEXT NOT NULL CHECK (engine IN ('aria2', 'yt-dlp', 'qbittorrent')),
            engine_task_id TEXT,
            file_name TEXT NOT NULL,
            status TEXT NOT NULL CHECK (
                status IN ('queued', 'running', 'paused', 'completed', 'failed', 'deleted')
            ),
            progress REAL NOT NULL DEFAULT 0 CHECK (progress >= 0 AND progress <= 100),
            speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (speed_bytes_per_sec >= 0),
            save_path TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            completed_at TEXT,
            error_message TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_download_tasks_created_at
            ON download_tasks (created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_download_tasks_status
            ON download_tasks (status);
        "#,
    )
}
