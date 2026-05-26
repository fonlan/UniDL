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
            name TEXT NOT NULL DEFAULT '',
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            executable_path TEXT,
            default_download_dir TEXT NOT NULL DEFAULT '',
            default_args TEXT NOT NULL DEFAULT '',
            connection_url TEXT,
            username TEXT,
            password TEXT,
            remote_path TEXT,
            supported_source_types TEXT NOT NULL DEFAULT '',
            preferred_domains TEXT NOT NULL DEFAULT '',
            tracker_subscription_url TEXT,
            trackers TEXT NOT NULL DEFAULT '',
            proxy_url TEXT,
            priority INTEGER NOT NULL DEFAULT 0,
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
            downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK (downloaded_bytes >= 0),
            total_bytes INTEGER NOT NULL DEFAULT 0 CHECK (total_bytes >= 0),
            save_path TEXT NOT NULL,
            engine_args TEXT NOT NULL DEFAULT '',
            selected_file_indexes TEXT,
            browser_cookies TEXT,
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
    migrate_engine_settings_name(connection)?;
    migrate_engine_settings_priority(connection)?;
    migrate_engine_settings_supported_source_types(connection)?;
    migrate_engine_settings_preferred_domains(connection)?;
    migrate_engine_settings_trackers(connection)?;
    migrate_engine_settings_proxy_url(connection)?;
    migrate_download_tasks_engine_settings_id(connection)?;
    migrate_download_tasks_engine_args(connection)?;
    migrate_download_tasks_size_fields(connection)?;
    migrate_download_tasks_selected_file_indexes(connection)?;
    migrate_download_tasks_browser_cookies(connection)?;
    seed_app_settings(connection)
}

fn migrate_download_tasks_size_fields(connection: &Connection) -> Result<(), rusqlite::Error> {
    if !has_column(connection, "download_tasks", "downloaded_bytes")? {
        connection.execute(
            "ALTER TABLE download_tasks ADD COLUMN downloaded_bytes INTEGER NOT NULL DEFAULT 0 CHECK (downloaded_bytes >= 0)",
            [],
        )?;
    }

    if !has_column(connection, "download_tasks", "total_bytes")? {
        connection.execute(
            "ALTER TABLE download_tasks ADD COLUMN total_bytes INTEGER NOT NULL DEFAULT 0 CHECK (total_bytes >= 0)",
            [],
        )?;
    }

    Ok(())
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
            name TEXT NOT NULL DEFAULT '',
            enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
            executable_path TEXT,
            default_download_dir TEXT NOT NULL DEFAULT '',
            default_args TEXT NOT NULL DEFAULT '',
            connection_url TEXT,
            username TEXT,
            password TEXT,
            remote_path TEXT,
            supported_source_types TEXT NOT NULL DEFAULT '',
            preferred_domains TEXT NOT NULL DEFAULT '',
            tracker_subscription_url TEXT,
            trackers TEXT NOT NULL DEFAULT '',
            proxy_url TEXT,
            priority INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

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
            priority,
            created_at,
            updated_at
        )
        SELECT
            engine,
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
            CASE engine
                WHEN 'aria2' THEN 'http,ftp,magnet,torrent'
                WHEN 'yt-dlp' THEN 'http,ftp'
                WHEN 'qbittorrent' THEN 'magnet,torrent'
                ELSE ''
            END,
            '',
            NULL,
            '',
            NULL,
            CASE engine
                WHEN 'aria2' THEN 0
                WHEN 'yt-dlp' THEN 1
                WHEN 'qbittorrent' THEN 2
                ELSE 3
            END,
            created_at,
            updated_at
        FROM engine_settings_legacy;

        DROP TABLE engine_settings_legacy;
        "#,
    )
}

fn migrate_engine_settings_name(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "name")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN name TEXT NOT NULL DEFAULT '';

        UPDATE engine_settings
        SET name = engine
        WHERE name = '';
        "#,
    )
}

fn migrate_engine_settings_priority(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "priority")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN priority INTEGER NOT NULL DEFAULT 0;

        UPDATE engine_settings
        SET priority = CASE engine
            WHEN 'aria2' THEN 0
            WHEN 'yt-dlp' THEN 1
            WHEN 'qbittorrent' THEN 2
            ELSE 3
        END;
        "#,
    )
}

fn migrate_engine_settings_supported_source_types(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "supported_source_types")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN supported_source_types TEXT NOT NULL DEFAULT '';

        UPDATE engine_settings
        SET supported_source_types = CASE engine
            WHEN 'aria2' THEN 'http,ftp,magnet,torrent'
            WHEN 'yt-dlp' THEN 'http,ftp'
            WHEN 'qbittorrent' THEN 'magnet,torrent'
            ELSE ''
        END;
        "#,
    )
}

fn migrate_engine_settings_preferred_domains(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "preferred_domains")? {
        return Ok(());
    }

    connection.execute(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN preferred_domains TEXT NOT NULL DEFAULT '';
        "#,
        [],
    )?;

    Ok(())
}

fn migrate_engine_settings_trackers(connection: &Connection) -> Result<(), rusqlite::Error> {
    if !has_column(connection, "engine_settings", "tracker_subscription_url")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN tracker_subscription_url TEXT;
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "trackers")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN trackers TEXT NOT NULL DEFAULT '';
            "#,
            [],
        )?;
    }

    Ok(())
}

fn migrate_engine_settings_proxy_url(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "engine_settings", "proxy_url")? {
        return Ok(());
    }

    connection.execute(
        r#"
        ALTER TABLE engine_settings
            ADD COLUMN proxy_url TEXT;
        "#,
        [],
    )?;

    Ok(())
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

fn migrate_download_tasks_selected_file_indexes(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if has_column(connection, "download_tasks", "selected_file_indexes")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE download_tasks
            ADD COLUMN selected_file_indexes TEXT;
        "#,
    )
}

fn migrate_download_tasks_browser_cookies(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "download_tasks", "browser_cookies")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE download_tasks
            ADD COLUMN browser_cookies TEXT;
        "#,
    )
}

fn seed_app_settings(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO app_settings (key, value) VALUES
            ('web_access_enabled', '0'),
            ('web_access_password', ''),
            ('web_access_url', 'http://127.0.0.1:18080'),
            ('private_download_domains', ''),
            ('app_proxy_url', '');
        "#,
    )
}
