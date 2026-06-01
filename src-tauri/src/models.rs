use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct ModelParseError {
    field: &'static str,
    value: String,
}

impl ModelParseError {
    fn new(field: &'static str, value: impl Into<String>) -> Self {
        Self {
            field,
            value: value.into(),
        }
    }
}

impl fmt::Display for ModelParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {} value: {}", self.field, self.value)
    }
}

impl Error for ModelParseError {}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum EngineKind {
    #[serde(rename = "aria2")]
    Aria2,
    #[serde(rename = "yt-dlp")]
    YtDlp,
    #[serde(rename = "qbittorrent")]
    QBittorrent,
}

impl EngineKind {
    pub fn from_db(value: &str) -> Result<Self, ModelParseError> {
        match value {
            "aria2" => Ok(Self::Aria2),
            "yt-dlp" => Ok(Self::YtDlp),
            "qbittorrent" => Ok(Self::QBittorrent),
            _ => Err(ModelParseError::new("engine", value)),
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Aria2 => "aria2",
            Self::YtDlp => "yt-dlp",
            Self::QBittorrent => "qbittorrent",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Http,
    Ftp,
    Magnet,
    Torrent,
}

impl SourceType {
    pub fn from_db(value: &str) -> Result<Self, ModelParseError> {
        match value {
            "http" => Ok(Self::Http),
            "ftp" => Ok(Self::Ftp),
            "magnet" => Ok(Self::Magnet),
            "torrent" => Ok(Self::Torrent),
            _ => Err(ModelParseError::new("source_type", value)),
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Ftp => "ftp",
            Self::Magnet => "magnet",
            Self::Torrent => "torrent",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileConflictAction {
    Prompt,
    Overwrite,
    Rename,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadDuplicateKind {
    SameSource,
    SameFinalUrl,
    SameSavePath,
    SameNameAndSize,
    SameTorrentInfoHash,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadDuplicateTaskState {
    Active,
    Completed,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
    Deleted,
}

impl DownloadStatus {
    pub fn from_db(value: &str) -> Result<Self, ModelParseError> {
        match value {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "deleted" => Ok(Self::Deleted),
            _ => Err(ModelParseError::new("status", value)),
        }
    }

    pub fn as_db(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Deleted => "deleted",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub source_type: SourceType,
    pub source: String,
    pub engine_settings_id: String,
    pub engine: EngineKind,
    pub engine_task_id: Option<String>,
    pub file_name: String,
    pub status: DownloadStatus,
    pub progress: f64,
    pub speed_bytes_per_sec: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub save_path: String,
    pub engine_args: String,
    pub selected_file_indexes: Option<Vec<i64>>,
    pub browser_cookies: Option<String>,
    pub http_referrer: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
    pub downloaded_file_missing: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDownloadTaskInput {
    pub source_type: SourceType,
    pub source: String,
    pub engine: EngineKind,
    pub engine_settings_id: Option<String>,
    pub file_name: String,
    pub save_path: String,
    pub engine_args: String,
    pub selected_file_indexes: Option<Vec<i64>>,
    pub browser_cookies: Option<String>,
    pub http_referrer: Option<String>,
    pub file_conflict_action: Option<FileConflictAction>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadFileConflict {
    pub file_name: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDuplicateMatch {
    pub kind: DownloadDuplicateKind,
    pub task: DownloadTask,
    pub task_state: DownloadDuplicateTaskState,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadDuplicateCheck {
    pub matches: Vec<DownloadDuplicateMatch>,
    pub local_file_conflict: Option<DownloadFileConflict>,
}

#[derive(Debug, Clone)]
pub struct NewDownloadTask {
    pub source_type: SourceType,
    pub source: String,
    pub engine_settings_id: String,
    pub engine: EngineKind,
    pub file_name: String,
    pub save_path: String,
    pub engine_args: String,
    pub selected_file_indexes: Option<Vec<i64>>,
    pub browser_cookies: Option<String>,
    pub http_referrer: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectoryEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSettings {
    pub id: String,
    pub engine: EngineKind,
    pub name: String,
    pub enabled: bool,
    pub executable_path: Option<String>,
    pub default_download_dir: String,
    pub default_args: String,
    pub connection_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub remote_path: Option<String>,
    pub supported_source_types: Vec<SourceType>,
    pub preferred_domains: Vec<String>,
    pub tracker_subscription_url: Option<String>,
    pub trackers: Vec<String>,
    pub proxy_url: Option<String>,
    pub user_agent: Option<String>,
    pub speed_limit_bytes_per_sec: i64,
    pub qbittorrent_download_limit_bytes_per_sec: i64,
    pub qbittorrent_upload_limit_bytes_per_sec: i64,
    pub qbittorrent_seed_ratio_limit: f64,
    pub qbittorrent_seed_time_limit_minutes: i64,
    pub aria2_enable_dht: bool,
    pub aria2_enable_dht6: bool,
    pub aria2_enable_peer_exchange: bool,
    pub aria2_enable_lpd: bool,
    pub aria2_bt_listen_port: i64,
    pub aria2_bt_max_peers: i64,
    pub aria2_max_connection_per_server: i64,
    pub aria2_split: i64,
    pub aria2_min_split_size: String,
    pub aria2_file_allocation: String,
    pub aria2_seed_time: i64,
    pub aria2_seed_ratio: f64,
    pub priority: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineSettingsInput {
    pub id: String,
    pub engine: EngineKind,
    pub name: String,
    pub enabled: bool,
    pub executable_path: Option<String>,
    pub default_download_dir: String,
    pub default_args: String,
    pub connection_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub remote_path: Option<String>,
    pub supported_source_types: Vec<SourceType>,
    pub preferred_domains: Vec<String>,
    pub tracker_subscription_url: Option<String>,
    pub trackers: Vec<String>,
    pub proxy_url: Option<String>,
    pub user_agent: Option<String>,
    pub speed_limit_bytes_per_sec: i64,
    pub qbittorrent_download_limit_bytes_per_sec: i64,
    pub qbittorrent_upload_limit_bytes_per_sec: i64,
    pub qbittorrent_seed_ratio_limit: f64,
    pub qbittorrent_seed_time_limit_minutes: i64,
    pub aria2_enable_dht: bool,
    pub aria2_enable_dht6: bool,
    pub aria2_enable_peer_exchange: bool,
    pub aria2_enable_lpd: bool,
    pub aria2_bt_listen_port: i64,
    pub aria2_bt_max_peers: i64,
    pub aria2_max_connection_per_server: i64,
    pub aria2_split: i64,
    pub aria2_min_split_size: String,
    pub aria2_file_allocation: String,
    pub aria2_seed_time: i64,
    pub aria2_seed_ratio: f64,
    pub priority: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallResult {
    pub settings: EngineSettings,
    pub version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme_mode: String,
    pub web_access_enabled: bool,
    pub web_access_password: String,
    pub web_access_url: String,
    pub private_download_domains: Vec<String>,
    pub app_proxy_url: String,
    pub torrent_file_association_enabled: bool,
    pub auto_start_enabled: bool,
    pub auto_start_minimized_to_tray: bool,
    pub close_to_tray_enabled: bool,
    pub download_completion_notification_enabled: bool,
    pub prevent_sleep_when_downloading_enabled: bool,
    pub prevent_sleep_when_web_access_enabled: bool,
    pub local_download_concurrency: i64,
    pub auto_clean_download_tasks_enabled: bool,
    pub auto_clean_download_tasks_days: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsInput {
    pub theme_mode: String,
    pub web_access_enabled: bool,
    pub web_access_password: String,
    pub web_access_url: String,
    pub private_download_domains: Vec<String>,
    pub app_proxy_url: String,
    pub torrent_file_association_enabled: bool,
    pub auto_start_enabled: bool,
    pub auto_start_minimized_to_tray: bool,
    pub close_to_tray_enabled: bool,
    pub download_completion_notification_enabled: bool,
    pub prevent_sleep_when_downloading_enabled: bool,
    pub prevent_sleep_when_web_access_enabled: bool,
    pub local_download_concurrency: i64,
    pub auto_clean_download_tasks_enabled: bool,
    pub auto_clean_download_tasks_days: i64,
}

pub fn supported_source_types(engine: EngineKind) -> Vec<SourceType> {
    match engine {
        EngineKind::Aria2 => vec![
            SourceType::Http,
            SourceType::Ftp,
            SourceType::Magnet,
            SourceType::Torrent,
        ],
        EngineKind::YtDlp => vec![SourceType::Http, SourceType::Ftp],
        EngineKind::QBittorrent => vec![SourceType::Magnet, SourceType::Torrent],
    }
}

pub fn engine_supports_source_type(engine: EngineKind, source_type: SourceType) -> bool {
    supported_source_types(engine).contains(&source_type)
}
