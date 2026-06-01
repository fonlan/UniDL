mod aria2;
mod qbittorrent;
mod ytdlp;

use std::{error::Error, path::PathBuf, process::Command};

use crate::{
    models::{DownloadStatus, DownloadTask, EngineKind, EngineSettings, RemoteDirectoryEntry},
    torrent_metadata::TorrentFileEntry,
};

#[cfg(test)]
use std::{fs, thread};

#[cfg(test)]
use serde_json::{json, Value};

#[cfg(test)]
use crate::models::SourceType;

#[cfg(test)]
use aria2::{
    aria2_download_options, aria2_params, aria2_rpc_url, refresh_task as refresh_aria2_task,
};

#[cfg(test)]
use ytdlp::{
    append_ytdlp_transfer_options, decode_ytdlp_stdout_line, is_ytdlp_format_temp_file_name,
    parse_ytdlp_destination_name, parse_ytdlp_progress, parse_ytdlp_speed,
    refresh_task as refresh_ytdlp_task, update_ytdlp_completion, update_ytdlp_progress,
    ytdlp_output_template,
};

pub(crate) use ytdlp::{apply_ytdlp_utf8_env, sanitize_ytdlp_output_name};

#[cfg(windows)]
pub(crate) fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn hide_console_window(_command: &mut Command) {}

pub struct EngineTaskState {
    pub status: DownloadStatus,
    pub progress: f64,
    pub speed_bytes_per_sec: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub engine_task_id: Option<String>,
    pub file_name: Option<String>,
    pub error_message: Option<String>,
}

impl EngineTaskState {
    pub(crate) fn running(engine_task_id: impl Into<String>) -> Self {
        Self {
            status: DownloadStatus::Running,
            progress: 0.0,
            speed_bytes_per_sec: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            engine_task_id: Some(engine_task_id.into()),
            file_name: None,
            error_message: None,
        }
    }
}

pub struct MagnetMetadata {
    pub name: Option<String>,
    pub files: Option<Vec<TorrentFileEntry>>,
}

pub fn add_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::add_task(settings, task),
        EngineKind::YtDlp => ytdlp::add_task(settings, task, database_path, false),
        EngineKind::QBittorrent => qbittorrent::add_task(settings, task),
    }
}

pub fn resolve_magnet_metadata(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<MagnetMetadata, Box<dyn Error>> {
    if !source
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("magnet:")
    {
        return Err("source must be a magnet link".into());
    }
    if save_path.trim().is_empty() {
        return Err("save path is required".into());
    }

    match settings.engine {
        EngineKind::Aria2 => aria2::resolve_magnet_metadata(settings, source, save_path),
        EngineKind::QBittorrent => {
            qbittorrent::resolve_magnet_metadata(settings, source, save_path)
        }
        EngineKind::YtDlp => Err("yt-dlp does not support magnet metadata".into()),
    }
}

pub fn task_torrent_files(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::task_torrent_files(settings, task),
        EngineKind::QBittorrent => qbittorrent::task_torrent_files(settings, task),
        EngineKind::YtDlp => Err("yt-dlp does not support torrent files".into()),
    }
}

pub fn update_task_file_selection(
    settings: &EngineSettings,
    task: &DownloadTask,
    selected_file_indexes: &[i64],
) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            aria2::update_task_file_selection(settings, task, selected_file_indexes)
        }
        EngineKind::QBittorrent => {
            qbittorrent::update_task_file_selection(settings, task, selected_file_indexes)
        }
        EngineKind::YtDlp => Err("yt-dlp does not support torrent file selection".into()),
    }
}

pub fn resolve_magnet_files(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    if !source
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("magnet:")
    {
        return Err("source must be a magnet link".into());
    }
    if save_path.trim().is_empty() {
        return Err("save path is required".into());
    }

    match settings.engine {
        EngineKind::Aria2 => aria2::resolve_magnet_files(settings, source, save_path),
        EngineKind::QBittorrent => qbittorrent::resolve_magnet_files(settings, source, save_path),
        EngineKind::YtDlp => Err("yt-dlp does not support magnet metadata".into()),
    }
}

pub fn list_remote_directories(
    settings: &EngineSettings,
    path: &str,
) -> Result<Vec<RemoteDirectoryEntry>, Box<dyn Error>> {
    match settings.engine {
        EngineKind::QBittorrent => qbittorrent::list_remote_directories(settings, path),
        EngineKind::Aria2 | EngineKind::YtDlp => Err(format!(
            "{:?} does not support remote directory browsing",
            settings.engine
        )
        .into()),
    }
}

pub fn test_connection(settings: &EngineSettings) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::test_connection(settings),
        EngineKind::QBittorrent => qbittorrent::test_connection(settings),
        EngineKind::YtDlp => Err("yt-dlp does not use a remote connection".into()),
    }
}

pub fn pause_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::pause_task(settings, task),
        EngineKind::YtDlp => ytdlp::pause_task(task),
        EngineKind::QBittorrent => qbittorrent::pause_task(settings, task),
    }
}

pub fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::resume_task(settings, task),
        EngineKind::YtDlp => ytdlp::resume_task(settings, task, database_path),
        EngineKind::QBittorrent => qbittorrent::resume_task(settings, task),
    }
}

pub fn delete_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    delete_files: bool,
) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::delete_task(settings, task),
        EngineKind::YtDlp => ytdlp::delete_task(task),
        EngineKind::QBittorrent => qbittorrent::delete_task(settings, task, delete_files),
    }
}

pub fn refresh_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => aria2::refresh_task(settings, task),
        EngineKind::YtDlp => ytdlp::refresh_task(task),
        EngineKind::QBittorrent => qbittorrent::refresh_task(settings, task),
    }
}

pub(crate) fn required_engine_task_id(task: &DownloadTask) -> Result<&str, Box<dyn Error>> {
    task.engine_task_id
        .as_deref()
        .ok_or_else(|| format!("task {} has no engine task id", task.id).into())
}

pub(crate) fn engine_proxy_url(settings: &EngineSettings) -> Option<&str> {
    settings
        .proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn engine_user_agent(settings: &EngineSettings) -> Option<&str> {
    settings
        .user_agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

pub(crate) fn engine_speed_limit_bytes_per_sec(settings: &EngineSettings) -> Option<i64> {
    (settings.speed_limit_bytes_per_sec > 0).then_some(settings.speed_limit_bytes_per_sec)
}

pub(crate) fn append_args(command: &mut Command, args: &str) {
    for arg in args.split_whitespace() {
        command.arg(arg);
    }
}

pub(crate) fn log_command(label: &str, command: &Command) {
    crate::logger::info(format!("{label}: {}", command_line(command)));
}

fn command_line(command: &Command) -> String {
    std::iter::once(shell_quote(&command.get_program().to_string_lossy()))
        .chain(
            command
                .get_args()
                .map(|arg| shell_quote(&arg.to_string_lossy())),
        )
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value.is_empty()
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '&' | '|' | '<' | '>' | '^'))
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

pub(crate) fn parse_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    value.and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_str().and_then(|text| text.parse::<i64>().ok()))
    })
}

pub(crate) fn parse_magnet_hash(source: &str) -> Option<String> {
    source
        .split('&')
        .find_map(|part| {
            part.strip_prefix("magnet:?xt=urn:btih:")
                .or_else(|| part.strip_prefix("xt=urn:btih:"))
        })
        .map(|hash| hash.to_ascii_lowercase())
}

pub(crate) fn bool_param(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}
#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    use super::*;
    use crate::repositories::DownloadTaskRepository;

    fn aria2_settings(default_args: &str) -> EngineSettings {
        EngineSettings {
            id: "aria2".to_string(),
            engine: EngineKind::Aria2,
            name: "aria2".to_string(),
            enabled: true,
            executable_path: None,
            default_download_dir: String::new(),
            default_args: default_args.to_string(),
            connection_url: Some("http://127.0.0.1:6800/jsonrpc".to_string()),
            username: None,
            password: None,
            remote_path: None,
            supported_source_types: vec![SourceType::Http],
            preferred_domains: Vec::new(),
            tracker_subscription_url: None,
            trackers: Vec::new(),
            proxy_url: None,
            user_agent: None,
            speed_limit_bytes_per_sec: 0,
            qbittorrent_download_limit_bytes_per_sec: 0,
            qbittorrent_upload_limit_bytes_per_sec: 0,
            qbittorrent_seed_ratio_limit: 0.0,
            qbittorrent_seed_time_limit_minutes: 0,
            aria2_enable_dht: true,
            aria2_enable_dht6: true,
            aria2_enable_peer_exchange: true,
            aria2_enable_lpd: true,
            aria2_bt_listen_port: 6881,
            aria2_bt_max_peers: 55,
            aria2_max_connection_per_server: 16,
            aria2_split: 16,
            aria2_min_split_size: "1M".to_string(),
            aria2_file_allocation: "none".to_string(),
            aria2_seed_time: 10,
            aria2_seed_ratio: 1.0,
            priority: 0,
            updated_at: String::new(),
        }
    }

    fn aria2_task() -> DownloadTask {
        DownloadTask {
            id: "task".to_string(),
            source_type: SourceType::Http,
            source: "http://example.test/file.bin".to_string(),
            engine_settings_id: "aria2".to_string(),
            engine: EngineKind::Aria2,
            engine_task_id: Some("gid".to_string()),
            file_name: "file.bin".to_string(),
            status: DownloadStatus::Running,
            progress: 12.0,
            speed_bytes_per_sec: 123,
            downloaded_bytes: 120,
            total_bytes: 1_000,
            save_path: "C:\\Downloads".to_string(),
            engine_args: String::new(),
            selected_file_indexes: None,
            browser_cookies: None,
            http_referrer: None,
            created_at: String::new(),
            completed_at: None,
            error_message: None,
            downloaded_file_missing: false,
        }
    }

    fn unreachable_aria2_settings() -> EngineSettings {
        let mut settings = aria2_settings("--continue=true");
        settings.connection_url = Some("http://127.0.0.1:1/jsonrpc".to_string());
        settings
    }

    #[test]
    fn aria2_params_prepends_rpc_secret_token() {
        let settings = aria2_settings("--continue=true --rpc-secret=secret-value");

        let params = aria2_params(&settings, json!(["gid"])).expect("params should build");

        assert_eq!(params, json!(["token:secret-value", "gid"]));
    }

    #[test]
    fn aria2_params_keeps_params_without_rpc_secret() {
        let settings = aria2_settings("--continue=true");

        let params = aria2_params(&settings, json!(["gid"])).expect("params should build");

        assert_eq!(params, json!(["gid"]));
    }

    #[test]
    fn aria2_rpc_url_appends_jsonrpc_path() {
        let mut settings = aria2_settings("--continue=true");
        settings.connection_url = Some("http://127.0.0.1:6800/".to_string());

        assert_eq!(aria2_rpc_url(&settings), "http://127.0.0.1:6800/jsonrpc");
    }

    #[test]
    fn aria2_download_options_uses_dialog_file_name() {
        let options = aria2_download_options(
            "C:\\Downloads",
            Some("renamed.bin"),
            "--continue=true --out=default.bin",
            "--split=4 --out=task.bin",
            None,
            None,
            None,
            None,
            55,
            16,
            16,
            "1M",
            "none",
            10,
            1.0,
            true,
            true,
            true,
            true,
        );

        assert_eq!(
            options.get("out").and_then(Value::as_str),
            Some("renamed.bin")
        );
    }

    #[test]
    fn aria2_download_options_uses_bt_discovery_toggles() {
        let options = aria2_download_options(
            "C:\\Downloads",
            None,
            "--enable-dht=true --enable-peer-exchange=true",
            "--bt-enable-lpd=false",
            None,
            None,
            None,
            None,
            55,
            16,
            16,
            "1M",
            "none",
            10,
            1.0,
            false,
            true,
            false,
            true,
        );

        assert_eq!(
            options.get("enable-dht").and_then(Value::as_str),
            Some("false")
        );
        assert_eq!(
            options.get("enable-dht6").and_then(Value::as_str),
            Some("true")
        );
        assert_eq!(
            options.get("enable-peer-exchange").and_then(Value::as_str),
            Some("false")
        );
        assert_eq!(
            options.get("bt-enable-lpd").and_then(Value::as_str),
            Some("true")
        );
    }

    #[test]
    fn aria2_download_options_uses_configured_transfer_options() {
        let options = aria2_download_options(
            "C:\\Downloads",
            None,
            "--max-connection-per-server=16 --split=16 --min-split-size=1M --file-allocation=none",
            "",
            None,
            None,
            None,
            None,
            55,
            8,
            4,
            "2M",
            "prealloc",
            10,
            1.0,
            true,
            true,
            true,
            true,
        );

        assert_eq!(
            options
                .get("max-connection-per-server")
                .and_then(Value::as_str),
            Some("8")
        );
        assert_eq!(options.get("split").and_then(Value::as_str), Some("4"));
        assert_eq!(
            options.get("min-split-size").and_then(Value::as_str),
            Some("2M")
        );
        assert_eq!(
            options.get("file-allocation").and_then(Value::as_str),
            Some("prealloc")
        );
    }

    #[test]
    fn aria2_download_options_uses_user_agent_and_speed_limit() {
        let options = aria2_download_options(
            "C:\\Downloads",
            None,
            "--user-agent=DefaultAgent --max-download-limit=1M",
            "",
            None,
            None,
            Some("UniDL Test Agent"),
            Some(524_288),
            55,
            16,
            16,
            "1M",
            "none",
            10,
            1.0,
            true,
            true,
            true,
            true,
        );

        assert_eq!(
            options.get("user-agent").and_then(Value::as_str),
            Some("UniDL Test Agent")
        );
        assert_eq!(
            options.get("max-download-limit").and_then(Value::as_str),
            Some("524288")
        );
    }

    #[test]
    fn pause_aria2_task_marks_failed_when_rpc_is_unavailable() {
        let state = pause_task(&unreachable_aria2_settings(), &aria2_task())
            .expect("unavailable aria2 should update local state");

        assert_eq!(state.status, DownloadStatus::Failed);
        assert_eq!(state.progress, 12.0);
        assert_eq!(state.speed_bytes_per_sec, 0);
        assert_eq!(state.engine_task_id.as_deref(), Some("gid"));
        assert_eq!(
            state.error_message.as_deref(),
            Some("aria2 rpc is unavailable")
        );
    }

    #[test]
    fn refresh_aria2_task_pauses_when_rpc_is_unavailable() {
        let state = refresh_aria2_task(&unreachable_aria2_settings(), &aria2_task())
            .expect("unavailable aria2 should keep resumable state");

        assert_eq!(state.status, DownloadStatus::Paused);
        assert_eq!(state.progress, 12.0);
        assert_eq!(state.speed_bytes_per_sec, 0);
        assert_eq!(state.downloaded_bytes, 120);
        assert_eq!(state.total_bytes, 1_000);
        assert_eq!(state.engine_task_id.as_deref(), Some("gid"));
        assert_eq!(state.error_message, None);
    }

    #[test]
    fn resume_paused_aria2_task_readds_when_gid_is_missing() {
        let (url, methods, server) = start_fake_aria2_missing_then_add();
        let mut settings = aria2_settings("--continue=true");
        settings.connection_url = Some(url);
        let mut task = aria2_task();
        task.status = DownloadStatus::Paused;

        let state = resume_task(&settings, &task, PathBuf::new())
            .expect("missing aria2 gid should be re-added");

        server.join().expect("fake aria2 should finish");
        assert_eq!(state.status, DownloadStatus::Running);
        assert_eq!(state.engine_task_id.as_deref(), Some("newgid"));
        assert_eq!(
            methods.lock().expect("methods should lock").as_slice(),
            ["aria2.unpause", "aria2.addUri"]
        );
    }

    #[test]
    fn delete_aria2_task_succeeds_when_rpc_is_unavailable() {
        delete_task(&unreachable_aria2_settings(), &aria2_task(), true)
            .expect("unavailable aria2 should not block local delete");
    }

    #[test]
    fn refresh_paused_ytdlp_task_keeps_pause_state() {
        let mut task = aria2_task();
        task.engine = EngineKind::YtDlp;
        task.status = DownloadStatus::Paused;
        task.engine_task_id = None;

        let state = refresh_ytdlp_task(&task).expect("paused yt-dlp task should refresh");

        assert_eq!(state.status, DownloadStatus::Paused);
        assert_eq!(state.progress, 12.0);
        assert_eq!(state.speed_bytes_per_sec, 0);
        assert_eq!(state.engine_task_id, None);
    }

    #[test]
    fn refresh_running_ytdlp_task_does_not_downgrade_on_dead_pid() {
        // Use a PID that is overwhelmingly unlikely to be alive. The bug being
        // guarded here is: refresh used to flip Running -> Failed whenever
        // tasklist reported the PID as gone, which clobbered live downloads
        // whose spawn thread was still writing progress.
        let mut task = aria2_task();
        task.engine = EngineKind::YtDlp;
        task.status = DownloadStatus::Running;
        task.engine_task_id = Some("4294967290".to_string());
        task.progress = 11.0;
        task.speed_bytes_per_sec = 1234;
        task.error_message = None;

        let state =
            refresh_ytdlp_task(&task).expect("running yt-dlp task should refresh without error");

        assert_eq!(state.status, DownloadStatus::Running);
        assert_eq!(state.progress, 11.0);
        assert_eq!(state.error_message, None);
    }

    /// Regression: when pause's `taskkill` failed,
    /// `services::pause_tasks::mark_failed` set the row to Failed but the
    /// underlying yt-dlp process was still alive. Refresh used to do
    /// `if pid_alive { Running }` and revive the user-visible failure
    /// back into Running -- producing the "Failed -> Running -> still
    /// downloading" flicker reported in 2026-05-26's session log
    /// (PID 38560, "failed to stop yt-dlp process 38560"). After this
    /// fix refresh is a pure status echo.
    #[test]
    fn refresh_failed_ytdlp_task_does_not_revive_when_pid_alive() {
        let mut task = aria2_task();
        task.engine = EngineKind::YtDlp;
        task.status = DownloadStatus::Failed;
        // The current process's PID is guaranteed alive while the test runs,
        // which is exactly the "taskkill failed but the process kept
        // running" scenario from the user report.
        task.engine_task_id = Some(std::process::id().to_string());
        task.progress = 42.0;
        task.speed_bytes_per_sec = 555;
        task.error_message = Some("failed to stop yt-dlp process".to_string());

        let state =
            refresh_ytdlp_task(&task).expect("failed yt-dlp task should refresh without error");

        assert_eq!(state.status, DownloadStatus::Failed);
        assert_eq!(state.progress, 42.0);
        assert_eq!(state.speed_bytes_per_sec, 0);
        assert_eq!(
            state.error_message.as_deref(),
            Some("failed to stop yt-dlp process")
        );
    }

    /// Regression: refresh must also leave Completed alone. Previously the
    /// "if pid_alive -> Running" branch could re-open a finished task if a
    /// new unrelated process happened to inherit the recycled PID before
    /// the row's engine_task_id was cleared.
    #[test]
    fn refresh_completed_ytdlp_task_keeps_completed() {
        let mut task = aria2_task();
        task.engine = EngineKind::YtDlp;
        task.status = DownloadStatus::Completed;
        task.engine_task_id = Some(std::process::id().to_string());
        task.progress = 100.0;
        task.speed_bytes_per_sec = 0;

        let state =
            refresh_ytdlp_task(&task).expect("completed yt-dlp task should refresh without error");

        assert_eq!(state.status, DownloadStatus::Completed);
        assert_eq!(state.progress, 100.0);
        assert_eq!(state.speed_bytes_per_sec, 0);
    }

    #[test]
    fn parse_ytdlp_progress_reads_percent_and_speed() {
        let progress = parse_ytdlp_progress("[download]  42.5% of 10.00MiB at 1.25MiB/s ETA 00:04")
            .expect("progress should parse");

        assert_eq!(progress.percent, 42.5);
        assert_eq!(progress.speed_bytes_per_sec, 1_310_720);
        assert_eq!(progress.downloaded_bytes, 4_456_448);
        assert_eq!(progress.total_bytes, 10_485_760);
    }

    #[test]
    fn parse_ytdlp_progress_reads_json_template() {
        let progress = parse_ytdlp_progress(
            r#"[UniDL:progress] {"status":"downloading","downloadedBytes":1048576,"totalBytes":0,"totalBytesEstimate":2097152,"speedBytesPerSec":524288.4,"percent":" 50.0%"}"#,
        )
        .expect("progress template should parse");

        assert_eq!(progress.percent, 50.0);
        assert_eq!(progress.speed_bytes_per_sec, 524_288);
        assert_eq!(progress.downloaded_bytes, 1_048_576);
        assert_eq!(progress.total_bytes, 2_097_152);
    }

    #[test]
    fn parse_ytdlp_progress_reads_separated_approximate_total() {
        let progress =
            parse_ytdlp_progress("[download]  42.5% of ~ 10.00MiB at 1.25MiB/s ETA 00:04")
                .expect("approximate total progress should parse");

        assert_eq!(progress.percent, 42.5);
        assert_eq!(progress.speed_bytes_per_sec, 1_310_720);
        assert_eq!(progress.downloaded_bytes, 4_456_448);
        assert_eq!(progress.total_bytes, 10_485_760);
    }

    #[test]
    fn parse_ytdlp_progress_keeps_speed_zero_when_missing() {
        let progress =
            parse_ytdlp_progress("[download]  42.5% of 10.00MiB").expect("progress should parse");

        assert_eq!(progress.percent, 42.5);
        assert_eq!(progress.speed_bytes_per_sec, 0);
        assert_eq!(progress.total_bytes, 10_485_760);
    }

    #[test]
    fn parse_ytdlp_speed_supports_decimal_units() {
        assert_eq!(parse_ytdlp_speed("128.5KB/s"), Some(128_500));
        assert_eq!(parse_ytdlp_speed("2MB/s"), Some(2_000_000));
    }

    #[test]
    fn decode_ytdlp_stdout_line_passes_through_utf8() {
        let input = "[Merger] Merging formats into \"鍥藉\".mp4".as_bytes();
        let decoded = decode_ytdlp_stdout_line(input);
        assert!(decoded.contains("鍥藉"));
    }

    #[cfg(windows)]
    #[test]
    fn decode_ytdlp_stdout_line_recovers_from_gbk_on_windows() {
        // "涓浗" in GBK is D6 D0 B9 FA 鈥?invalid as UTF-8, valid as CP936/GBK.
        let bytes = [0xD6, 0xD0, 0xB9, 0xFA];
        let decoded = decode_ytdlp_stdout_line(&bytes);
        assert_eq!(decoded, "涓浗");
    }

    #[test]
    fn parse_ytdlp_destination_name_recognises_download_merger_and_extract_lines() {
        assert_eq!(
            parse_ytdlp_destination_name("[download] Destination: C:\\Downloads\\My Video.mp4")
                .as_deref(),
            Some("My Video.mp4")
        );
        assert_eq!(
            parse_ytdlp_destination_name(
                "[Merger] Merging formats into \"C:\\Downloads\\My Video.mp4\""
            )
            .as_deref(),
            Some("My Video.mp4")
        );
        assert_eq!(
            parse_ytdlp_destination_name("[ExtractAudio] Destination: /tmp/Track.m4a").as_deref(),
            Some("Track.m4a")
        );
        assert_eq!(
            parse_ytdlp_destination_name("[download]  42.5% of 10.00MiB at 1.25MiB/s ETA 00:04"),
            None
        );
    }

    #[test]
    fn ytdlp_transfer_options_append_user_agent_and_limit_rate() {
        let mut command = Command::new("yt-dlp");
        append_ytdlp_transfer_options(&mut command, Some("UniDL Test Agent"), Some(524_288));

        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(
            args,
            vec![
                "--user-agent".to_string(),
                "UniDL Test Agent".to_string(),
                "--limit-rate".to_string(),
                "524288".to_string(),
            ]
        );
    }

    #[test]
    fn is_ytdlp_format_temp_file_name_matches_per_format_streams_only() {
        assert!(is_ytdlp_format_temp_file_name("My Video.f137.mp4"));
        assert!(is_ytdlp_format_temp_file_name("My Video.f140.m4a"));
        assert!(!is_ytdlp_format_temp_file_name("My Video.mp4"));
        assert!(!is_ytdlp_format_temp_file_name("My Video.fancy.mp4"));
        assert!(!is_ytdlp_format_temp_file_name("My Video"));
        assert!(!is_ytdlp_format_temp_file_name(""));
    }

    #[test]
    fn ytdlp_output_template_appends_ext_placeholder_only_when_missing() {
        assert_eq!(ytdlp_output_template("Page Title"), "Page Title.%(ext)s");
        assert_eq!(ytdlp_output_template("video.mp4"), "video.mp4");
        assert_eq!(
            ytdlp_output_template("Page Title.%(ext)s"),
            "Page Title.%(ext)s"
        );
    }

    #[test]
    fn ytdlp_output_template_replaces_windows_invalid_filename_chars() {
        assert_eq!(
            ytdlp_output_template("4K 娴锋磱鐢熺墿濂囪 | 鎺㈢储:娴锋磱 - YouTube"),
            "4K 娴锋磱鐢熺墿濂囪 _ 鎺㈢储_娴锋磱 - YouTube.%(ext)s"
        );
    }

    #[test]
    fn delete_completed_ytdlp_task_does_not_kill_finished_process() {
        let settings = EngineSettings {
            id: "yt-dlp".to_string(),
            engine: EngineKind::YtDlp,
            name: "yt-dlp".to_string(),
            enabled: true,
            executable_path: Some("yt-dlp.exe".to_string()),
            default_download_dir: String::new(),
            default_args: String::new(),
            connection_url: None,
            username: None,
            password: None,
            remote_path: None,
            supported_source_types: vec![SourceType::Http, SourceType::Ftp],
            preferred_domains: Vec::new(),
            tracker_subscription_url: None,
            trackers: Vec::new(),
            proxy_url: None,
            user_agent: None,
            speed_limit_bytes_per_sec: 0,
            qbittorrent_download_limit_bytes_per_sec: 0,
            qbittorrent_upload_limit_bytes_per_sec: 0,
            qbittorrent_seed_ratio_limit: 0.0,
            qbittorrent_seed_time_limit_minutes: 0,
            aria2_enable_dht: true,
            aria2_enable_dht6: true,
            aria2_enable_peer_exchange: true,
            aria2_enable_lpd: true,
            aria2_bt_listen_port: 6881,
            aria2_bt_max_peers: 55,
            aria2_max_connection_per_server: 16,
            aria2_split: 16,
            aria2_min_split_size: "1M".to_string(),
            aria2_file_allocation: "none".to_string(),
            aria2_seed_time: 10,
            aria2_seed_ratio: 1.0,
            priority: 0,
            updated_at: String::new(),
        };
        let mut task = aria2_task();
        task.engine = EngineKind::YtDlp;
        task.status = DownloadStatus::Completed;

        delete_task(&settings, &task, false).expect("completed yt-dlp task should delete");
    }

    #[test]
    fn delete_completed_aria2_task_removes_download_result() {
        let (url, methods, server) = start_fake_aria2("complete", 2);
        let mut settings = aria2_settings("--continue=true");
        settings.connection_url = Some(url);

        delete_task(&settings, &aria2_task(), false).expect("completed aria2 task should delete");

        server.join().expect("fake aria2 should finish");
        assert_eq!(
            methods.lock().expect("methods should lock").as_slice(),
            ["aria2.tellStatus", "aria2.removeDownloadResult"]
        );
    }

    #[test]
    fn delete_active_aria2_task_removes_active_download() {
        let (url, methods, server) = start_fake_aria2("active", 2);
        let mut settings = aria2_settings("--continue=true");
        settings.connection_url = Some(url);

        delete_task(&settings, &aria2_task(), true).expect("active aria2 task should delete");

        server.join().expect("fake aria2 should finish");
        assert_eq!(
            methods.lock().expect("methods should lock").as_slice(),
            ["aria2.tellStatus", "aria2.remove"]
        );
    }

    fn start_fake_aria2(
        status: &'static str,
        expected_requests: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake aria2 should bind");
        let address = listener
            .local_addr()
            .expect("fake aria2 should have address");
        let methods = Arc::new(Mutex::new(Vec::new()));
        let server_methods = Arc::clone(&methods);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(expected_requests) {
                let mut stream = stream.expect("fake aria2 stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .expect("request should include body");
                let request: Value = serde_json::from_str(body).expect("request should be json");
                let method = request
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("request should include method")
                    .to_string();
                server_methods
                    .lock()
                    .expect("methods should lock")
                    .push(method.clone());
                let response = if method == "aria2.tellStatus" {
                    json!({"jsonrpc": "2.0", "id": "unidl", "result": {"status": status}})
                } else {
                    json!({"jsonrpc": "2.0", "id": "unidl", "result": "OK"})
                };
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
            }
        });

        (format!("http://{address}/jsonrpc"), methods, server)
    }

    fn start_fake_aria2_missing_then_add(
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake aria2 should bind");
        let address = listener
            .local_addr()
            .expect("fake aria2 should have address");
        let methods = Arc::new(Mutex::new(Vec::new()));
        let server_methods = Arc::clone(&methods);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(2) {
                let mut stream = stream.expect("fake aria2 stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]);
                let body = request
                    .split("\r\n\r\n")
                    .nth(1)
                    .expect("request should include body");
                let request: Value = serde_json::from_str(body).expect("request should be json");
                let method = request
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("request should include method")
                    .to_string();
                server_methods
                    .lock()
                    .expect("methods should lock")
                    .push(method.clone());
                let response = if method == "aria2.unpause" {
                    json!({"jsonrpc": "2.0", "id": "unidl", "error": {"code": 1, "message": "GID#gid is not found"}})
                } else {
                    json!({"jsonrpc": "2.0", "id": "unidl", "result": "newgid"})
                };
                let body = response.to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("response should write");
            }
        });

        (format!("http://{address}/jsonrpc"), methods, server)
    }

    /// Inserts a yt-dlp task row directly so the spawn-thread DB writers can be
    /// exercised without actually running yt-dlp.
    fn insert_ytdlp_task_row(
        connection: &rusqlite::Connection,
        id: &str,
        engine_task_id: &str,
        status: DownloadStatus,
        progress: f64,
        error_message: Option<&str>,
    ) {
        connection
            .execute(
                r#"
                INSERT INTO download_tasks (
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
                    error_message
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                "#,
                rusqlite::params![
                    id,
                    SourceType::Http.as_db(),
                    "http://example.test/video.mp4",
                    "yt-dlp",
                    EngineKind::YtDlp.as_db(),
                    engine_task_id,
                    "video.mp4",
                    status.as_db(),
                    progress,
                    0_i64,
                    0_i64,
                    0_i64,
                    "C:\\Downloads",
                    error_message,
                ],
            )
            .expect("yt-dlp task row should insert");
    }

    fn read_task_row(connection: &rusqlite::Connection, id: &str) -> DownloadTask {
        DownloadTaskRepository::new(connection)
            .get_by_id(id)
            .expect("task should load")
    }

    fn temp_engine_database_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "unidl-engine-test-{}.sqlite3",
            uuid::Uuid::new_v4()
        ))
    }

    /// Regression: pausing a Running yt-dlp task triggers taskkill, which makes
    /// the spawn thread's stdout/stderr pipes flush their last buffered progress
    /// lines. Each of those lines used to call `update_ytdlp_progress` and
    /// clobber the user's freshly-written `Paused` back to `Running`, producing
    /// the "Failed -> Running -> Failed" status flicker users reported. The
    /// guard makes those late writes a no-op when the DB no longer says
    /// Running.
    #[test]
    fn update_ytdlp_progress_skips_when_task_is_paused() {
        let database_path = temp_engine_database_path();
        let connection = crate::db::connect_path(database_path.clone()).expect("db should migrate");
        insert_ytdlp_task_row(
            &connection,
            "paused-task",
            "1234",
            DownloadStatus::Paused,
            42.0,
            None,
        );
        drop(connection);

        update_ytdlp_progress(&database_path, "paused-task", "1234", 88.0, 999, 880, 1_000)
            .expect("late progress write should succeed but skip");

        let connection = rusqlite::Connection::open(&database_path).expect("db should open");
        let task = read_task_row(&connection, "paused-task");
        assert_eq!(task.status, DownloadStatus::Paused);
        assert_eq!(task.progress, 42.0);
        assert_eq!(task.speed_bytes_per_sec, 0);
        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    /// Regression: when pause kills yt-dlp the spawn thread's `child.wait()`
    /// reports a non-zero exit and is about to write `update_ytdlp_completion`
    /// with status=Failed. That used to overwrite the user's Paused with
    /// Failed (often after the DB had already settled into Paused), producing
    /// the brief "Failed" flash. The guard ensures the late completion write
    /// is a no-op when the user has already paused.
    #[test]
    fn update_ytdlp_completion_skips_when_task_is_paused() {
        let database_path = temp_engine_database_path();
        let connection = crate::db::connect_path(database_path.clone()).expect("db should migrate");
        insert_ytdlp_task_row(
            &connection,
            "paused-task",
            "1234",
            DownloadStatus::Paused,
            42.0,
            None,
        );
        drop(connection);

        update_ytdlp_completion(
            &database_path,
            "paused-task",
            "1234",
            DownloadStatus::Failed,
            Some("yt-dlp exited with failure"),
        )
        .expect("late completion write should succeed but skip");

        let connection = rusqlite::Connection::open(&database_path).expect("db should open");
        let task = read_task_row(&connection, "paused-task");
        assert_eq!(task.status, DownloadStatus::Paused);
        assert_eq!(task.progress, 42.0);
        assert_eq!(task.error_message, None);
        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    /// Regression: if the user pauses then immediately resumes, a fresh spawn
    /// will rewrite engine_task_id with the new PID. The previous spawn's
    /// thread eventually finishes its `child.wait()` and tries to write Failed
    /// (because the old child was killed). That stale write must NOT clobber
    /// the new Running session.
    #[test]
    fn update_ytdlp_completion_skips_when_engine_task_id_changed() {
        let database_path = temp_engine_database_path();
        let connection = crate::db::connect_path(database_path.clone()).expect("db should migrate");
        // Simulate the post-resume state: status=Running, engine_task_id=NEW.
        insert_ytdlp_task_row(
            &connection,
            "resumed-task",
            "5678",
            DownloadStatus::Running,
            42.0,
            None,
        );
        drop(connection);

        // The old spawn (pid=1234) finally reports its Failed exit.
        update_ytdlp_completion(
            &database_path,
            "resumed-task",
            "1234",
            DownloadStatus::Failed,
            Some("old spawn was killed"),
        )
        .expect("stale completion write should succeed but skip");

        let connection = rusqlite::Connection::open(&database_path).expect("db should open");
        let task = read_task_row(&connection, "resumed-task");
        assert_eq!(task.status, DownloadStatus::Running);
        assert_eq!(task.engine_task_id.as_deref(), Some("5678"));
        assert_eq!(task.error_message, None);
        drop(connection);
        let _ = fs::remove_file(database_path);
    }

    /// Sanity: when the spawn thread is still the live one (pid matches and
    /// status is Running), normal progress writes go through.
    #[test]
    fn update_ytdlp_progress_writes_when_owned_and_running() {
        let database_path = temp_engine_database_path();
        let connection = crate::db::connect_path(database_path.clone()).expect("db should migrate");
        insert_ytdlp_task_row(
            &connection,
            "live-task",
            "1234",
            DownloadStatus::Running,
            10.0,
            None,
        );
        drop(connection);

        update_ytdlp_progress(&database_path, "live-task", "1234", 55.5, 4096, 555, 1_000)
            .expect("live progress should write");

        let connection = rusqlite::Connection::open(&database_path).expect("db should open");
        let task = read_task_row(&connection, "live-task");
        assert_eq!(task.status, DownloadStatus::Running);
        assert_eq!(task.progress, 55.5);
        assert_eq!(task.speed_bytes_per_sec, 4096);
        assert_eq!(task.downloaded_bytes, 555);
        assert_eq!(task.total_bytes, 1_000);
        drop(connection);
        let _ = fs::remove_file(database_path);
    }
}
