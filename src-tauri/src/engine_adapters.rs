use std::{
    collections::VecDeque,
    error::Error,
    fs,
    io::{BufRead, BufReader, Read},
    num::NonZeroU32,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

use base64::{engine::general_purpose, Engine as _};
use reqwest::blocking::{multipart, Client};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    logger,
    models::{DownloadStatus, DownloadTask, EngineKind, EngineSettings, SourceType},
    repositories::DownloadTaskRepository,
    torrent_metadata::read_torrent_info_hash,
};

const ARIA2_FAST_DEFAULT_ARGS: &str = "--continue=true --max-connection-per-server=16 --split=16 --min-split-size=1M --file-allocation=none";
const YTDLP_FAST_DEFAULT_ARGS: &str =
    "--newline --no-playlist --js-runtimes node --concurrent-fragments 8";

pub struct EngineTaskState {
    pub status: DownloadStatus,
    pub progress: f64,
    pub speed_bytes_per_sec: i64,
    pub downloaded_bytes: i64,
    pub total_bytes: i64,
    pub engine_task_id: Option<String>,
    pub error_message: Option<String>,
}

impl EngineTaskState {
    fn running(engine_task_id: impl Into<String>) -> Self {
        Self {
            status: DownloadStatus::Running,
            progress: 0.0,
            speed_bytes_per_sec: 0,
            downloaded_bytes: 0,
            total_bytes: 0,
            engine_task_id: Some(engine_task_id.into()),
            error_message: None,
        }
    }
}

pub fn add_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => add_aria2_task(settings, task),
        EngineKind::YtDlp => add_ytdlp_task(settings, task, database_path, false),
        EngineKind::QBittorrent => add_qbittorrent_task(settings, task),
    }
}

pub fn test_connection(settings: &EngineSettings) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            aria2_rpc(settings, "aria2.getVersion", json!([]))?;
            Ok(())
        }
        EngineKind::QBittorrent => {
            qbittorrent_client(settings)?;
            Ok(())
        }
        EngineKind::YtDlp => Err("yt-dlp does not use a remote connection".into()),
    }
}

pub fn pause_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            let gid = required_engine_task_id(task)?;
            if let Err(error) = aria2_rpc(settings, "aria2.pause", json!([gid])) {
                if aria2_unavailable(error.as_ref()) {
                    return Ok(EngineTaskState {
                        status: DownloadStatus::Failed,
                        progress: task.progress,
                        speed_bytes_per_sec: 0,
                        downloaded_bytes: task.downloaded_bytes,
                        total_bytes: task.total_bytes,
                        engine_task_id: task.engine_task_id.clone(),
                        error_message: Some("aria2 rpc is unavailable".to_string()),
                    });
                }
                return Err(error);
            }
            Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                downloaded_bytes: task.downloaded_bytes,
                total_bytes: task.total_bytes,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
        EngineKind::YtDlp => {
            terminate_process(ytdlp_pid(task)?)?;
            Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                downloaded_bytes: task.downloaded_bytes,
                total_bytes: task.total_bytes,
                engine_task_id: None,
                error_message: None,
            })
        }
        EngineKind::QBittorrent => {
            let hash = required_engine_task_id(task)?;
            let client = qbittorrent_client(settings)?;
            qbittorrent_post(&client, settings, "torrents/pause", &[("hashes", hash)])?;
            Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                downloaded_bytes: task.downloaded_bytes,
                total_bytes: task.total_bytes,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
    }
}

pub fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            if task.status == DownloadStatus::Paused {
                let gid = required_engine_task_id(task)?;
                match aria2_rpc(settings, "aria2.unpause", json!([gid])) {
                    Ok(_) => Ok(EngineTaskState {
                        status: DownloadStatus::Running,
                        progress: task.progress,
                        speed_bytes_per_sec: 0,
                        downloaded_bytes: task.downloaded_bytes,
                        total_bytes: task.total_bytes,
                        engine_task_id: task.engine_task_id.clone(),
                        error_message: None,
                    }),
                    Err(error) if aria2_resumable_after_restart(error.as_ref()) => {
                        add_aria2_task(settings, task)
                    }
                    Err(error) => Err(error),
                }
            } else {
                add_aria2_task(settings, task)
            }
        }
        EngineKind::YtDlp => add_ytdlp_task(settings, task, database_path, true),
        EngineKind::QBittorrent => {
            let hash = required_engine_task_id(task)?;
            let client = qbittorrent_client(settings)?;
            qbittorrent_post(&client, settings, "torrents/resume", &[("hashes", hash)])?;
            Ok(EngineTaskState {
                status: DownloadStatus::Running,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                downloaded_bytes: task.downloaded_bytes,
                total_bytes: task.total_bytes,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
    }
}

pub fn delete_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    delete_files: bool,
) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            let gid = required_engine_task_id(task)?;
            let method = match aria2_delete_method(settings, gid) {
                Ok(method) => method,
                Err(error) if aria2_unavailable(error.as_ref()) => return Ok(()),
                Err(error) => return Err(error),
            };
            if let Err(error) = aria2_rpc(settings, method, json!([gid])) {
                if !aria2_unavailable(error.as_ref()) {
                    return Err(error);
                }
            }
        }
        EngineKind::YtDlp => {
            if !matches!(
                task.status,
                DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Paused
            ) {
                let pid = required_engine_task_id(task)?;
                run_windows_command("taskkill", &["/PID", pid, "/T", "/F"])?;
            }
        }
        EngineKind::QBittorrent => {
            let hash = required_engine_task_id(task)?;
            let client = qbittorrent_client(settings)?;
            qbittorrent_post(
                &client,
                settings,
                "torrents/delete",
                &[("hashes", hash), ("deleteFiles", bool_param(delete_files))],
            )?;
        }
    }
    Ok(())
}

pub fn refresh_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => refresh_aria2_task(settings, task),
        EngineKind::YtDlp => refresh_ytdlp_task(task),
        EngineKind::QBittorrent => refresh_qbittorrent_task(settings, task),
    }
}

fn add_aria2_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    start_aria2_process(settings, &task.save_path)?;

    let output_file_name = match task.source_type {
        SourceType::Http | SourceType::Ftp => Some(task.file_name.trim()),
        SourceType::Magnet | SourceType::Torrent => None,
    };
    let options = aria2_download_options(
        &task.save_path,
        output_file_name,
        &settings.default_args,
        &task.engine_args,
        task.selected_file_indexes.as_deref(),
    );
    let result = match task.source_type {
        SourceType::Torrent => {
            let torrent = fs::read(&task.source)?;
            let torrent_base64 = general_purpose::STANDARD.encode(torrent);
            aria2_rpc(
                settings,
                "aria2.addTorrent",
                json!([torrent_base64, [], options]),
            )?
        }
        _ => aria2_rpc(settings, "aria2.addUri", json!([[task.source], options]))?,
    };

    let gid = result
        .as_str()
        .ok_or("aria2 did not return a task gid")?
        .to_string();
    Ok(EngineTaskState::running(gid))
}

fn start_aria2_process(settings: &EngineSettings, save_path: &str) -> Result<(), Box<dyn Error>> {
    let executable = settings.executable_path.as_deref().unwrap_or("").trim();
    if executable.is_empty() {
        return Ok(());
    }

    let mut command = Command::new(executable);
    command
        .arg("--enable-rpc=true")
        .arg("--rpc-listen-all=false")
        .arg("--rpc-listen-port=6800")
        .args(ARIA2_FAST_DEFAULT_ARGS.split_whitespace())
        .arg(format!("--dir={}", save_path))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    append_args(&mut command, &settings.default_args);
    log_command("starting aria2", &command);
    command.spawn().map_err(|error| {
        logger::error(format!("aria2 spawn failed: {error}"));
        error
    })?;
    Ok(())
}

fn refresh_aria2_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let gid = required_engine_task_id(task)?;
    let result = match aria2_rpc(
        settings,
        "aria2.tellStatus",
        json!([
            gid,
            [
                "gid",
                "status",
                "totalLength",
                "completedLength",
                "downloadSpeed",
                "errorMessage"
            ]
        ]),
    ) {
        Ok(result) => result,
        Err(error) if aria2_resumable_after_restart(error.as_ref()) => {
            return Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                downloaded_bytes: task.downloaded_bytes,
                total_bytes: task.total_bytes,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            });
        }
        Err(error) => return Err(error),
    };

    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("error");
    let total = parse_i64(result.get("totalLength")).unwrap_or(0);
    let completed = parse_i64(result.get("completedLength")).unwrap_or(0);
    let progress = if total > 0 {
        (completed as f64 / total as f64) * 100.0
    } else {
        task.progress
    };
    let speed = parse_i64(result.get("downloadSpeed")).unwrap_or(0);
    let error_message = result
        .get("errorMessage")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    Ok(EngineTaskState {
        status: match status {
            "complete" => DownloadStatus::Completed,
            "active" => DownloadStatus::Running,
            "waiting" => DownloadStatus::Queued,
            "paused" => DownloadStatus::Paused,
            "removed" => DownloadStatus::Deleted,
            _ => DownloadStatus::Failed,
        },
        progress,
        speed_bytes_per_sec: speed,
        downloaded_bytes: completed,
        total_bytes: total,
        engine_task_id: task.engine_task_id.clone(),
        error_message,
    })
}

fn aria2_rpc(
    settings: &EngineSettings,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn Error>> {
    let url = aria2_rpc_url(settings);
    let client = Client::new();
    let params = aria2_params(settings, params)?;
    let response = client
        .post(&url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "unidl",
            "method": method,
            "params": params,
        }))
        .send()?;
    if !response.status().is_success() {
        return Err(format!("aria2 rpc failed: {}", response.status()).into());
    }

    let body: Value = response.json()?;
    if let Some(error) = body.get("error") {
        return Err(format!("aria2 rpc error: {}", error).into());
    }
    body.get("result")
        .cloned()
        .ok_or_else(|| "aria2 rpc response missing result".into())
}

fn aria2_rpc_url(settings: &EngineSettings) -> String {
    let url = settings
        .connection_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:6800/jsonrpc")
        .trim();
    if url.ends_with("/jsonrpc") {
        url.to_string()
    } else {
        format!("{}/jsonrpc", url.trim_end_matches('/'))
    }
}

fn aria2_params(settings: &EngineSettings, params: Value) -> Result<Value, Box<dyn Error>> {
    let mut params = params
        .as_array()
        .cloned()
        .ok_or("aria2 rpc params must be an array")?;
    if let Some(secret) = aria2_rpc_secret(&settings.default_args) {
        params.insert(0, Value::String(format!("token:{secret}")));
    }
    Ok(Value::Array(params))
}

fn aria2_rpc_secret(args: &str) -> Option<String> {
    let mut parts = args.split_whitespace();
    while let Some(part) = parts.next() {
        if let Some(secret) = part.strip_prefix("--rpc-secret=") {
            return Some(secret.to_string());
        }
        if part == "--rpc-secret" {
            return parts.next().map(ToOwned::to_owned);
        }
    }
    None
}

fn aria2_download_options(
    save_path: &str,
    output_file_name: Option<&str>,
    default_args: &str,
    task_args: &str,
    selected_file_indexes: Option<&[i64]>,
) -> Value {
    let mut options = serde_json::Map::new();
    options.insert("dir".to_string(), Value::String(save_path.to_string()));
    append_aria2_options(&mut options, ARIA2_FAST_DEFAULT_ARGS);
    append_aria2_options(&mut options, default_args);
    append_aria2_options(&mut options, task_args);
    if let Some(output_file_name) = output_file_name.filter(|value| !value.is_empty()) {
        options.insert(
            "out".to_string(),
            Value::String(output_file_name.to_string()),
        );
    }
    if let Some(selected_file_indexes) = selected_file_indexes {
        if !selected_file_indexes.is_empty() {
            options.insert(
                "select-file".to_string(),
                Value::String(format_select_file_indexes(selected_file_indexes)),
            );
        }
    }
    Value::Object(options)
}

fn format_select_file_indexes(indexes: &[i64]) -> String {
    indexes
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn append_aria2_options(options: &mut serde_json::Map<String, Value>, args: &str) {
    let parts = args.split_whitespace().collect::<Vec<_>>();
    let mut index = 0;
    while index < parts.len() {
        let Some(option) = parts[index].strip_prefix("--") else {
            index += 1;
            continue;
        };
        if let Some((key, value)) = option.split_once('=') {
            if aria2_download_option_allowed(key) {
                options.insert(key.to_string(), Value::String(value.to_string()));
            }
            index += 1;
            continue;
        }
        if let Some(value) = parts
            .get(index + 1)
            .filter(|value| !value.starts_with("--"))
        {
            if aria2_download_option_allowed(option) {
                options.insert(option.to_string(), Value::String((*value).to_string()));
            }
            index += 2;
            continue;
        }
        index += 1;
    }
}

fn aria2_download_option_allowed(key: &str) -> bool {
    !matches!(
        key,
        "dir" | "enable-rpc" | "rpc-listen-all" | "rpc-listen-port" | "rpc-secret"
    )
}

fn bool_param(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn aria2_delete_method(
    settings: &EngineSettings,
    gid: &str,
) -> Result<&'static str, Box<dyn Error>> {
    let result = aria2_rpc(settings, "aria2.tellStatus", json!([gid, ["status"]]))?;
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .ok_or("aria2 status response missing status")?;
    match status {
        "active" | "waiting" | "paused" => Ok("aria2.remove"),
        "complete" | "error" | "removed" => Ok("aria2.removeDownloadResult"),
        _ => Err(format!("unsupported aria2 task status: {status}").into()),
    }
}

fn aria2_unavailable(error: &(dyn Error + 'static)) -> bool {
    error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_connect() || error.is_timeout())
}

fn aria2_resumable_after_restart(error: &(dyn Error + 'static)) -> bool {
    aria2_unavailable(error) || aria2_task_missing(error)
}

fn aria2_task_missing(error: &(dyn Error + 'static)) -> bool {
    error.to_string().contains("is not found")
}

fn add_qbittorrent_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let mut form = multipart::Form::new()
        .text("savepath", task.save_path.clone())
        .text(
            "paused",
            if task
                .selected_file_indexes
                .as_ref()
                .is_some_and(|indexes| !indexes.is_empty())
            {
                "true"
            } else {
                "false"
            },
        );

    if task.source_type == SourceType::Torrent && Path::new(&task.source).exists() {
        let bytes = fs::read(&task.source)?;
        form = form.part(
            "torrents",
            multipart::Part::bytes(bytes).file_name(task.file_name.clone()),
        );
    } else {
        form = form.text("urls", task.source.clone());
    }

    let response = client
        .post(qbittorrent_url(settings, "torrents/add")?)
        .multipart(form)
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent add failed: {}", response.status()).into());
    }

    let hash = if task.source_type == SourceType::Torrent && Path::new(&task.source).exists() {
        read_torrent_info_hash(&task.source)?
    } else {
        parse_magnet_hash(&task.source).unwrap_or_else(|| task.source.clone())
    };

    if let Some(indexes) = task.selected_file_indexes.as_deref() {
        if !indexes.is_empty() {
            qbittorrent_select_files(&client, settings, &hash, indexes)?;
        }
    }

    qbittorrent_post(&client, settings, "torrents/resume", &[("hashes", &hash)])?;

    Ok(EngineTaskState::running(hash))
}

fn refresh_qbittorrent_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let hash = required_engine_task_id(task)?;
    let client = qbittorrent_client(settings)?;
    let response = client
        .get(qbittorrent_url(settings, "torrents/info")?)
        .query(&[("hashes", hash)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent status failed: {}", response.status()).into());
    }

    let torrents: Vec<QbTorrentInfo> = response.json()?;
    let torrent = torrents
        .first()
        .ok_or_else(|| format!("qBittorrent task not found: {}", hash))?;
    Ok(EngineTaskState {
        status: qbittorrent_state(&torrent.state, torrent.progress),
        progress: torrent.progress * 100.0,
        speed_bytes_per_sec: torrent.dlspeed,
        downloaded_bytes: torrent.downloaded,
        total_bytes: torrent.total_size,
        engine_task_id: task.engine_task_id.clone(),
        error_message: None,
    })
}

#[derive(Deserialize)]
struct QbTorrentInfo {
    progress: f64,
    dlspeed: i64,
    downloaded: i64,
    total_size: i64,
    state: String,
}

fn qbittorrent_state(state: &str, progress: f64) -> DownloadStatus {
    if progress >= 1.0 {
        return DownloadStatus::Completed;
    }

    let state = state.to_ascii_lowercase();
    if state.contains("paused") {
        DownloadStatus::Paused
    } else if state.contains("error") || state.contains("missing") {
        DownloadStatus::Failed
    } else {
        DownloadStatus::Running
    }
}

fn qbittorrent_client(settings: &EngineSettings) -> Result<Client, Box<dyn Error>> {
    let client = Client::builder().cookie_store(true).build()?;
    let username = settings.username.as_deref().unwrap_or("");
    let password = settings.password.as_deref().unwrap_or("");
    let response = client
        .post(qbittorrent_url(settings, "auth/login")?)
        .form(&[("username", username), ("password", password)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent login failed: {}", response.status()).into());
    }
    let body = response.text()?;
    if body.trim() != "Ok." {
        return Err("qBittorrent login failed: invalid username or password".into());
    }
    Ok(client)
}

fn qbittorrent_post(
    client: &Client,
    settings: &EngineSettings,
    endpoint: &str,
    form: &[(&str, &str)],
) -> Result<(), Box<dyn Error>> {
    let response = client
        .post(qbittorrent_url(settings, endpoint)?)
        .form(form)
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent {} failed: {}", endpoint, response.status()).into());
    }
    Ok(())
}

#[derive(Deserialize)]
struct QbTorrentFileInfo {
    index: i64,
}

fn qbittorrent_select_files(
    client: &Client,
    settings: &EngineSettings,
    hash: &str,
    selected_file_indexes: &[i64],
) -> Result<(), Box<dyn Error>> {
    let files: Vec<QbTorrentFileInfo> = qbittorrent_get_files(client, settings, hash)?;
    let selected: std::collections::HashSet<i64> = selected_file_indexes
        .iter()
        .map(|index| index.saturating_sub(1))
        .collect();
    let selected_ids = files
        .iter()
        .filter(|file| selected.contains(&file.index))
        .map(|file| file.index.to_string())
        .collect::<Vec<_>>()
        .join("|");
    let unselected_ids = files
        .iter()
        .filter(|file| !selected.contains(&file.index))
        .map(|file| file.index.to_string())
        .collect::<Vec<_>>()
        .join("|");
    if !unselected_ids.is_empty() {
        qbittorrent_post(
            client,
            settings,
            "torrents/filePrio",
            &[("hash", hash), ("id", &unselected_ids), ("priority", "0")],
        )?;
    }
    if !selected_ids.is_empty() {
        qbittorrent_post(
            client,
            settings,
            "torrents/filePrio",
            &[("hash", hash), ("id", &selected_ids), ("priority", "1")],
        )?;
    }
    Ok(())
}

fn qbittorrent_get_files(
    client: &Client,
    settings: &EngineSettings,
    hash: &str,
) -> Result<Vec<QbTorrentFileInfo>, Box<dyn Error>> {
    let response = client
        .get(qbittorrent_url(settings, "torrents/files")?)
        .query(&[("hash", hash)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent files failed: {}", response.status()).into());
    }
    Ok(response.json()?)
}

fn qbittorrent_url(settings: &EngineSettings, endpoint: &str) -> Result<String, Box<dyn Error>> {
    let base = settings
        .connection_url
        .as_deref()
        .ok_or("qBittorrent connection url is required")?
        .trim_end_matches('/');
    Ok(format!("{}/api/v2/{}", base, endpoint))
}

fn add_ytdlp_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
    force_continue: bool,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let executable = settings
        .executable_path
        .as_deref()
        .ok_or("yt-dlp executable path is required")?;
    let mut command = Command::new(executable);
    let cookie_path = write_ytdlp_cookie_file(task)?;
    append_args(&mut command, YTDLP_FAST_DEFAULT_ARGS);
    append_args(&mut command, &settings.default_args);
    append_args(&mut command, &task.engine_args);
    if force_continue {
        command.arg("--continue");
    } else {
        command.arg("--force-overwrites");
    }
    if let Some(cookie_path) = &cookie_path {
        command.arg("--cookies").arg(cookie_path);
    }
    command
        .arg("--output")
        .arg(ytdlp_output_template(&task.file_name))
        .arg("-P")
        .arg(&task.save_path)
        .arg(&task.source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    log_command("starting yt-dlp", &command);
    let mut child = command.spawn().map_err(|error| {
        logger::error(format!(
            "yt-dlp spawn failed: task_id={}, error={error}",
            task.id
        ));
        error
    })?;
    let pid = child.id().to_string();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let task_id = task.id.clone();
    let progress_task_id = task_id.clone();
    let progress_database_path = database_path.clone();
    thread::spawn(move || {
        let output_lines = Arc::new(Mutex::new(VecDeque::new()));
        let stdout_reader = stdout.map(|output| {
            spawn_ytdlp_progress_reader(
                output,
                progress_database_path.clone(),
                progress_task_id.clone(),
                Arc::clone(&output_lines),
            )
        });
        let stderr_reader = stderr.map(|output| {
            spawn_ytdlp_progress_reader(
                output,
                progress_database_path,
                progress_task_id,
                Arc::clone(&output_lines),
            )
        });

        let (status, exit_message) = match child.wait() {
            Ok(exit) if exit.success() => (
                DownloadStatus::Completed,
                format!("yt-dlp exited successfully: task_id={task_id}, status={exit}"),
            ),
            Ok(exit) => (
                DownloadStatus::Failed,
                format!("yt-dlp exited with failure: task_id={task_id}, status={exit}"),
            ),
            Err(error) => (
                DownloadStatus::Failed,
                format!("yt-dlp wait failed: task_id={task_id}, error={error}"),
            ),
        };
        if let Some(reader) = stdout_reader {
            let _ = reader.join();
        }
        if let Some(reader) = stderr_reader {
            let _ = reader.join();
        }
        if let Some(cookie_path) = cookie_path {
            let _ = fs::remove_file(cookie_path);
        }
        let error_message = if status == DownloadStatus::Failed {
            ytdlp_error_message(&output_lines)
        } else {
            None
        };
        if let Some(error_message) = &error_message {
            logger::error(format!("{exit_message}\nyt-dlp output:\n{error_message}"));
        } else {
            logger::info(exit_message);
        }
        let _ = update_ytdlp_completion(&database_path, &task_id, status, error_message.as_deref());
    });

    Ok(EngineTaskState::running(pid))
}

fn spawn_ytdlp_progress_reader(
    output: impl Read + Send + 'static,
    database_path: PathBuf,
    task_id: String,
    output_lines: Arc<Mutex<VecDeque<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(output);
        let mut bytes = Vec::new();
        while let Ok(read) = reader.read_until(b'\n', &mut bytes) {
            if read == 0 {
                break;
            }
            while matches!(bytes.last(), Some(b'\n' | b'\r')) {
                bytes.pop();
            }
            let line = String::from_utf8_lossy(&bytes).into_owned();
            if let Some(progress) = parse_ytdlp_progress(&line) {
                let _ = update_ytdlp_progress(
                    &database_path,
                    &task_id,
                    progress.percent,
                    progress.speed_bytes_per_sec,
                    progress.downloaded_bytes,
                    progress.total_bytes,
                );
            }
            remember_ytdlp_output_line(&output_lines, line);
            bytes.clear();
        }
    })
}

fn remember_ytdlp_output_line(output_lines: &Arc<Mutex<VecDeque<String>>>, line: String) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    if let Ok(mut output_lines) = output_lines.lock() {
        if output_lines.len() == 20 {
            output_lines.pop_front();
        }
        output_lines.push_back(line.to_string());
    }
}

fn ytdlp_error_message(output_lines: &Arc<Mutex<VecDeque<String>>>) -> Option<String> {
    output_lines
        .lock()
        .ok()
        .map(|output_lines| output_lines.iter().cloned().collect::<Vec<_>>().join("\n"))
        .filter(|message| !message.trim().is_empty())
}

fn ytdlp_output_template(file_name: &str) -> String {
    let file_name = sanitize_ytdlp_output_name(file_name.trim());
    if Path::new(&file_name).extension().is_some() || file_name.ends_with("%(ext)s") {
        file_name
    } else {
        format!("{file_name}.%(ext)s")
    }
}

pub(crate) fn sanitize_ytdlp_output_name(file_name: &str) -> String {
    file_name
        .chars()
        .map(|value| match value {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            value if value.is_control() => '_',
            value => value,
        })
        .collect::<String>()
        .trim_end_matches([' ', '.'])
        .to_string()
}

fn write_ytdlp_cookie_file(task: &DownloadTask) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let Some(cookies) = task.browser_cookies.as_deref() else {
        return Ok(None);
    };
    if cookies.trim().is_empty() {
        return Ok(None);
    }

    let cookie_path = std::env::temp_dir().join(format!("{}.cookies.txt", task.id));
    fs::write(&cookie_path, cookies)?;
    Ok(Some(cookie_path))
}

fn refresh_ytdlp_task(task: &DownloadTask) -> Result<EngineTaskState, Box<dyn Error>> {
    if task.status == DownloadStatus::Paused {
        return Ok(EngineTaskState {
            status: DownloadStatus::Paused,
            progress: task.progress,
            speed_bytes_per_sec: 0,
            downloaded_bytes: task.downloaded_bytes,
            total_bytes: task.total_bytes,
            engine_task_id: None,
            error_message: task.error_message.clone(),
        });
    }

    let pid = required_engine_task_id(task)?;
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output()?;
    let running = String::from_utf8_lossy(&output.stdout).contains(pid);

    Ok(EngineTaskState {
        status: if running {
            DownloadStatus::Running
        } else if task.status == DownloadStatus::Completed {
            DownloadStatus::Completed
        } else {
            DownloadStatus::Failed
        },
        progress: if running || task.status != DownloadStatus::Completed {
            task.progress
        } else {
            100.0
        },
        speed_bytes_per_sec: if running { task.speed_bytes_per_sec } else { 0 },
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
        engine_task_id: task.engine_task_id.clone(),
        error_message: if running || task.status == DownloadStatus::Completed {
            task.error_message.clone()
        } else {
            Some("yt-dlp process is not running".to_string())
        },
    })
}

#[derive(Debug, PartialEq)]
struct YtDlpProgress {
    percent: f64,
    speed_bytes_per_sec: i64,
    downloaded_bytes: i64,
    total_bytes: i64,
}

fn parse_ytdlp_progress(line: &str) -> Option<YtDlpProgress> {
    let parts = line.split_whitespace().collect::<Vec<_>>();
    let percent = parts
        .iter()
        .find_map(|part| part.strip_suffix('%')?.parse::<f64>().ok())?;
    let speed_bytes_per_sec = parts
        .windows(2)
        .find_map(|window| {
            if window[0] == "at" {
                parse_ytdlp_speed(window[1])
            } else {
                None
            }
        })
        .unwrap_or(0);
    let total_bytes = parts
        .windows(2)
        .find_map(|window| {
            if window[0] == "of" {
                parse_ytdlp_byte_value(window[1])
            } else {
                None
            }
        })
        .unwrap_or(0);
    let downloaded_bytes = if total_bytes > 0 {
        ((total_bytes as f64) * percent / 100.0).round() as i64
    } else {
        0
    };

    Some(YtDlpProgress {
        percent,
        speed_bytes_per_sec,
        downloaded_bytes,
        total_bytes,
    })
}

fn parse_ytdlp_speed(value: &str) -> Option<i64> {
    parse_ytdlp_byte_value(value.strip_suffix("/s")?)
}

fn parse_ytdlp_byte_value(value: &str) -> Option<i64> {
    let value = value.trim_start_matches('~');
    let unit_start = value
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(value.len());
    let number = value[..unit_start].parse::<f64>().ok()?;
    let unit = &value[unit_start..];
    let multiplier = match unit {
        "" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "B" => 1.0,
        "KB" => 1000.0,
        "MB" => 1000.0 * 1000.0,
        "GB" => 1000.0 * 1000.0 * 1000.0,
        _ => return None,
    };

    Some((number * multiplier).round() as i64)
}

fn update_ytdlp_progress(
    database_path: &Path,
    task_id: &str,
    progress: f64,
    speed_bytes_per_sec: i64,
    downloaded_bytes: i64,
    total_bytes: i64,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    repository.update_engine_state(
        task_id,
        DownloadStatus::Running,
        progress,
        speed_bytes_per_sec,
        downloaded_bytes,
        total_bytes,
        None,
        None,
    )?;
    Ok(())
}

fn update_ytdlp_completion(
    database_path: &Path,
    task_id: &str,
    status: DownloadStatus,
    error_message: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    let task = repository.get_by_id(task_id)?;
    let progress = if status == DownloadStatus::Completed {
        100.0
    } else {
        task.progress
    };
    let downloaded_bytes = if status == DownloadStatus::Completed && task.total_bytes > 0 {
        task.total_bytes
    } else {
        task.downloaded_bytes
    };
    repository.update_engine_state(
        task_id,
        status,
        progress,
        0,
        downloaded_bytes,
        task.total_bytes,
        None,
        error_message,
    )?;
    Ok(())
}

fn append_args(command: &mut Command, args: &str) {
    for arg in args.split_whitespace() {
        command.arg(arg);
    }
}

fn log_command(label: &str, command: &Command) {
    logger::info(format!("{label}: {}", command_line(command)));
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

fn parse_i64(value: Option<&Value>) -> Option<i64> {
    value.and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_str().and_then(|text| text.parse::<i64>().ok()))
    })
}

fn parse_magnet_hash(source: &str) -> Option<String> {
    source
        .split('&')
        .find_map(|part| {
            part.strip_prefix("magnet:?xt=urn:btih:")
                .or_else(|| part.strip_prefix("xt=urn:btih:"))
        })
        .map(|hash| hash.to_ascii_lowercase())
}

fn required_engine_task_id(task: &DownloadTask) -> Result<&str, Box<dyn Error>> {
    task.engine_task_id
        .as_deref()
        .ok_or_else(|| format!("task {} has no engine task id", task.id).into())
}

fn ytdlp_pid(task: &DownloadTask) -> Result<NonZeroU32, Box<dyn Error>> {
    let pid = required_engine_task_id(task)?
        .parse::<NonZeroU32>()
        .map_err(|_| format!("task {} has invalid yt-dlp pid", task.id))?;
    Ok(pid)
}

#[cfg(windows)]
fn terminate_process(pid: NonZeroU32) -> Result<(), Box<dyn Error>> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("failed to stop yt-dlp process {}", pid).into())
    }
}

#[cfg(not(windows))]
fn terminate_process(_pid: NonZeroU32) -> Result<(), Box<dyn Error>> {
    Err("yt-dlp pause is only supported on Windows".into())
}

fn run_windows_command(program: &str, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed with status {}", program, status).into())
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
            created_at: String::new(),
            completed_at: None,
            error_message: None,
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
        );

        assert_eq!(
            options.get("out").and_then(Value::as_str),
            Some("renamed.bin")
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
    fn parse_ytdlp_progress_reads_percent_and_speed() {
        let progress = parse_ytdlp_progress("[download]  42.5% of 10.00MiB at 1.25MiB/s ETA 00:04")
            .expect("progress should parse");

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
            ytdlp_output_template("4K 海洋生物奇观 | 探索:海洋 - YouTube"),
            "4K 海洋生物奇观 _ 探索_海洋 - YouTube.%(ext)s"
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
}
