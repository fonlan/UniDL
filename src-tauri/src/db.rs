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
            user_agent TEXT,
            speed_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (speed_limit_bytes_per_sec >= 0),
            qbittorrent_download_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_download_limit_bytes_per_sec >= 0),
            qbittorrent_upload_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_upload_limit_bytes_per_sec >= 0),
            qbittorrent_seed_ratio_limit REAL NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_ratio_limit >= 0),
            qbittorrent_seed_time_limit_minutes INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_time_limit_minutes >= 0),
            aria2_enable_dht INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht IN (0, 1)),
            aria2_enable_dht6 INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht6 IN (0, 1)),
            aria2_enable_peer_exchange INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_peer_exchange IN (0, 1)),
            aria2_enable_lpd INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_lpd IN (0, 1)),
            aria2_bt_listen_port INTEGER NOT NULL DEFAULT 6881 CHECK (aria2_bt_listen_port >= 1 AND aria2_bt_listen_port <= 65535),
            aria2_bt_max_peers INTEGER NOT NULL DEFAULT 55 CHECK (aria2_bt_max_peers >= 0),
            aria2_max_connection_per_server INTEGER NOT NULL DEFAULT 16 CHECK (aria2_max_connection_per_server >= 1),
            aria2_split INTEGER NOT NULL DEFAULT 16 CHECK (aria2_split >= 1),
            aria2_min_split_size TEXT NOT NULL DEFAULT '1M',
            aria2_file_allocation TEXT NOT NULL DEFAULT 'none',
            aria2_seed_time INTEGER NOT NULL DEFAULT 10 CHECK (aria2_seed_time >= 0),
            aria2_seed_ratio REAL NOT NULL DEFAULT 1.0 CHECK (aria2_seed_ratio >= 0),
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
            http_referrer TEXT,
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
    migrate_engine_settings_transfer_options(connection)?;
    migrate_engine_settings_qbittorrent_task_options(connection)?;
    migrate_engine_settings_aria2_bt_options(connection)?;
    migrate_download_tasks_engine_settings_id(connection)?;
    migrate_download_tasks_engine_args(connection)?;
    migrate_download_tasks_size_fields(connection)?;
    migrate_download_tasks_selected_file_indexes(connection)?;
    migrate_download_tasks_browser_cookies(connection)?;
    migrate_download_tasks_http_referrer(connection)?;
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
            user_agent TEXT,
            speed_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (speed_limit_bytes_per_sec >= 0),
            qbittorrent_download_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_download_limit_bytes_per_sec >= 0),
            qbittorrent_upload_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_upload_limit_bytes_per_sec >= 0),
            qbittorrent_seed_ratio_limit REAL NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_ratio_limit >= 0),
            qbittorrent_seed_time_limit_minutes INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_time_limit_minutes >= 0),
            aria2_enable_dht INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht IN (0, 1)),
            aria2_enable_dht6 INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht6 IN (0, 1)),
            aria2_enable_peer_exchange INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_peer_exchange IN (0, 1)),
            aria2_enable_lpd INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_lpd IN (0, 1)),
            aria2_bt_listen_port INTEGER NOT NULL DEFAULT 6881 CHECK (aria2_bt_listen_port >= 1 AND aria2_bt_listen_port <= 65535),
            aria2_bt_max_peers INTEGER NOT NULL DEFAULT 55 CHECK (aria2_bt_max_peers >= 0),
            aria2_max_connection_per_server INTEGER NOT NULL DEFAULT 16 CHECK (aria2_max_connection_per_server >= 1),
            aria2_split INTEGER NOT NULL DEFAULT 16 CHECK (aria2_split >= 1),
            aria2_min_split_size TEXT NOT NULL DEFAULT '1M',
            aria2_file_allocation TEXT NOT NULL DEFAULT 'none',
            aria2_seed_time INTEGER NOT NULL DEFAULT 10 CHECK (aria2_seed_time >= 0),
            aria2_seed_ratio REAL NOT NULL DEFAULT 1.0 CHECK (aria2_seed_ratio >= 0),
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
            user_agent,
            speed_limit_bytes_per_sec,
            aria2_enable_dht,
            aria2_enable_dht6,
            aria2_enable_peer_exchange,
            aria2_enable_lpd,
            aria2_max_connection_per_server,
            aria2_split,
            aria2_min_split_size,
            aria2_file_allocation,
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
            NULL,
            0,
            1,
            1,
            1,
            1,
            16,
            16,
            '1M',
            'none',
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

fn migrate_engine_settings_transfer_options(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if !has_column(connection, "engine_settings", "user_agent")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN user_agent TEXT;
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "speed_limit_bytes_per_sec")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN speed_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (speed_limit_bytes_per_sec >= 0);
            "#,
            [],
        )?;
    }

    Ok(())
}

fn migrate_engine_settings_qbittorrent_task_options(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if !has_column(
        connection,
        "engine_settings",
        "qbittorrent_download_limit_bytes_per_sec",
    )? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN qbittorrent_download_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_download_limit_bytes_per_sec >= 0);
            "#,
            [],
        )?;
    }

    if !has_column(
        connection,
        "engine_settings",
        "qbittorrent_upload_limit_bytes_per_sec",
    )? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN qbittorrent_upload_limit_bytes_per_sec INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_upload_limit_bytes_per_sec >= 0);
            "#,
            [],
        )?;
    }

    if !has_column(
        connection,
        "engine_settings",
        "qbittorrent_seed_ratio_limit",
    )? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN qbittorrent_seed_ratio_limit REAL NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_ratio_limit >= 0);
            "#,
            [],
        )?;
    }

    if !has_column(
        connection,
        "engine_settings",
        "qbittorrent_seed_time_limit_minutes",
    )? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN qbittorrent_seed_time_limit_minutes INTEGER NOT NULL DEFAULT 0 CHECK (qbittorrent_seed_time_limit_minutes >= 0);
            "#,
            [],
        )?;
    }

    Ok(())
}

fn migrate_engine_settings_aria2_bt_options(
    connection: &Connection,
) -> Result<(), rusqlite::Error> {
    if !has_column(connection, "engine_settings", "aria2_enable_dht")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_enable_dht INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht IN (0, 1));
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_enable_dht6")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_enable_dht6 INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_dht6 IN (0, 1));
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_enable_peer_exchange")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_enable_peer_exchange INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_peer_exchange IN (0, 1));
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_enable_lpd")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_enable_lpd INTEGER NOT NULL DEFAULT 1 CHECK (aria2_enable_lpd IN (0, 1));
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_bt_listen_port")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_bt_listen_port INTEGER NOT NULL DEFAULT 6881 CHECK (aria2_bt_listen_port >= 1 AND aria2_bt_listen_port <= 65535);
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_bt_max_peers")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_bt_max_peers INTEGER NOT NULL DEFAULT 55 CHECK (aria2_bt_max_peers >= 0);
            "#,
            [],
        )?;
    }

    if !has_column(
        connection,
        "engine_settings",
        "aria2_max_connection_per_server",
    )? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_max_connection_per_server INTEGER NOT NULL DEFAULT 16 CHECK (aria2_max_connection_per_server >= 1);
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_split")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_split INTEGER NOT NULL DEFAULT 16 CHECK (aria2_split >= 1);
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_min_split_size")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_min_split_size TEXT NOT NULL DEFAULT '1M';
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_file_allocation")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_file_allocation TEXT NOT NULL DEFAULT 'none';
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_seed_time")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_seed_time INTEGER NOT NULL DEFAULT 10 CHECK (aria2_seed_time >= 0);
            "#,
            [],
        )?;
    }

    if !has_column(connection, "engine_settings", "aria2_seed_ratio")? {
        connection.execute(
            r#"
            ALTER TABLE engine_settings
                ADD COLUMN aria2_seed_ratio REAL NOT NULL DEFAULT 1.0 CHECK (aria2_seed_ratio >= 0);
            "#,
            [],
        )?;
    }

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

fn migrate_download_tasks_http_referrer(connection: &Connection) -> Result<(), rusqlite::Error> {
    if has_column(connection, "download_tasks", "http_referrer")? {
        return Ok(());
    }

    connection.execute_batch(
        r#"
        ALTER TABLE download_tasks
            ADD COLUMN http_referrer TEXT;
        "#,
    )
}

fn seed_app_settings(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        r#"
        INSERT OR IGNORE INTO app_settings (key, value) VALUES
            ('theme_mode', 'light'),
            ('web_access_enabled', '0'),
            ('web_access_password', ''),
            ('web_access_url', 'http://127.0.0.1:18080'),
            ('private_download_domains', ''),
            ('app_proxy_url', ''),
            ('torrent_file_association_enabled', '1'),
            ('auto_start_enabled', '0'),
            ('auto_start_minimized_to_tray', '0'),
            ('close_to_tray_enabled', '0'),
            ('download_completion_notification_enabled', '0'),
            ('prevent_sleep_when_downloading_enabled', '0'),
            ('prevent_sleep_when_web_access_enabled', '0'),
            ('local_download_concurrency', '5'),
            ('auto_clean_download_tasks_enabled', '0'),
            ('auto_clean_download_tasks_days', '365');
        "#,
    )
}
