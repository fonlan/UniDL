use std::{
    error::Error,
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
};

use base64::{engine::general_purpose, Engine as _};
use reqwest::blocking::{multipart, Client};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    models::{DownloadStatus, DownloadTask, EngineKind, EngineSettings, SourceType},
    repositories::DownloadTaskRepository,
};

pub struct EngineTaskState {
    pub status: DownloadStatus,
    pub progress: f64,
    pub speed_bytes_per_sec: i64,
    pub engine_task_id: Option<String>,
    pub error_message: Option<String>,
}

impl EngineTaskState {
    fn running(engine_task_id: impl Into<String>) -> Self {
        Self {
            status: DownloadStatus::Running,
            progress: 0.0,
            speed_bytes_per_sec: 0,
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
        EngineKind::YtDlp => add_ytdlp_task(settings, task, database_path),
        EngineKind::QBittorrent => add_qbittorrent_task(settings, task),
    }
}

pub fn pause_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            let gid = required_engine_task_id(task)?;
            aria2_rpc(settings, "aria2.pause", json!([gid]))?;
            Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
        EngineKind::YtDlp => {
            let pid = required_engine_task_id(task)?;
            run_windows_command(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    &format!("Suspend-Process -Id {}", pid),
                ],
            )?;
            Ok(EngineTaskState {
                status: DownloadStatus::Paused,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                engine_task_id: task.engine_task_id.clone(),
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
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
    }
}

pub fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            let gid = required_engine_task_id(task)?;
            aria2_rpc(settings, "aria2.unpause", json!([gid]))?;
            Ok(EngineTaskState {
                status: DownloadStatus::Running,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
        EngineKind::YtDlp => {
            let pid = required_engine_task_id(task)?;
            run_windows_command(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    &format!("Resume-Process -Id {}", pid),
                ],
            )?;
            Ok(EngineTaskState {
                status: DownloadStatus::Running,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
        EngineKind::QBittorrent => {
            let hash = required_engine_task_id(task)?;
            let client = qbittorrent_client(settings)?;
            qbittorrent_post(&client, settings, "torrents/resume", &[("hashes", hash)])?;
            Ok(EngineTaskState {
                status: DownloadStatus::Running,
                progress: task.progress,
                speed_bytes_per_sec: 0,
                engine_task_id: task.engine_task_id.clone(),
                error_message: None,
            })
        }
    }
}

pub fn delete_task(settings: &EngineSettings, task: &DownloadTask) -> Result<(), Box<dyn Error>> {
    match settings.engine {
        EngineKind::Aria2 => {
            let gid = required_engine_task_id(task)?;
            aria2_rpc(settings, "aria2.remove", json!([gid]))?;
        }
        EngineKind::YtDlp => {
            let pid = required_engine_task_id(task)?;
            run_windows_command("taskkill", &["/PID", pid, "/T", "/F"])?;
        }
        EngineKind::QBittorrent => {
            let hash = required_engine_task_id(task)?;
            let client = qbittorrent_client(settings)?;
            qbittorrent_post(
                &client,
                settings,
                "torrents/delete",
                &[("hashes", hash), ("deleteFiles", "false")],
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

    let options = json!({
        "dir": task.save_path,
    });
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
        .arg("--continue=true")
        .arg(format!("--dir={}", save_path))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    append_args(&mut command, &settings.default_args);
    command.spawn()?;
    Ok(())
}

fn refresh_aria2_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let gid = required_engine_task_id(task)?;
    let result = aria2_rpc(
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
    )?;

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
        engine_task_id: task.engine_task_id.clone(),
        error_message,
    })
}

fn aria2_rpc(
    settings: &EngineSettings,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn Error>> {
    let url = settings
        .connection_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:6800/jsonrpc");
    let client = Client::new();
    let response = client
        .post(url)
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

fn add_qbittorrent_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let mut form = multipart::Form::new()
        .text("savepath", task.save_path.clone())
        .text("paused", "false");

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

    Ok(EngineTaskState::running(
        parse_magnet_hash(&task.source).unwrap_or_else(|| task.source.clone()),
    ))
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
        engine_task_id: task.engine_task_id.clone(),
        error_message: None,
    })
}

#[derive(Deserialize)]
struct QbTorrentInfo {
    progress: f64,
    dlspeed: i64,
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
) -> Result<EngineTaskState, Box<dyn Error>> {
    let executable = settings
        .executable_path
        .as_deref()
        .ok_or("yt-dlp executable path is required")?;
    let mut command = Command::new(executable);
    append_args(&mut command, &settings.default_args);
    append_args(&mut command, &task.engine_args);
    command
        .arg("--newline")
        .arg("-P")
        .arg(&task.save_path)
        .arg(&task.source)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let pid = child.id().to_string();
    let stderr = child.stderr.take();
    let task_id = task.id.clone();
    thread::spawn(move || {
        if let Some(stderr) = stderr {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(progress) = parse_ytdlp_progress(&line) {
                    let _ = update_ytdlp_progress(&database_path, &task_id, progress);
                }
            }
        }

        let status = match child.wait() {
            Ok(exit) if exit.success() => DownloadStatus::Completed,
            Ok(_) | Err(_) => DownloadStatus::Failed,
        };
        let _ = update_ytdlp_completion(&database_path, &task_id, status);
    });

    Ok(EngineTaskState::running(pid))
}

fn refresh_ytdlp_task(task: &DownloadTask) -> Result<EngineTaskState, Box<dyn Error>> {
    let pid = required_engine_task_id(task)?;
    let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output()?;
    let running = String::from_utf8_lossy(&output.stdout).contains(pid);

    Ok(EngineTaskState {
        status: if running {
            DownloadStatus::Running
        } else if task.status == DownloadStatus::Failed {
            DownloadStatus::Failed
        } else {
            DownloadStatus::Completed
        },
        progress: if running { task.progress } else { 100.0 },
        speed_bytes_per_sec: if running { task.speed_bytes_per_sec } else { 0 },
        engine_task_id: task.engine_task_id.clone(),
        error_message: task.error_message.clone(),
    })
}

fn parse_ytdlp_progress(line: &str) -> Option<f64> {
    line.split_whitespace()
        .find(|part| part.ends_with('%'))
        .and_then(|part| part.trim_end_matches('%').parse::<f64>().ok())
}

fn update_ytdlp_progress(
    database_path: &Path,
    task_id: &str,
    progress: f64,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    repository.update_engine_state(task_id, DownloadStatus::Running, progress, 0, None, None)?;
    Ok(())
}

fn update_ytdlp_completion(
    database_path: &Path,
    task_id: &str,
    status: DownloadStatus,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    repository.update_engine_state(task_id, status, 100.0, 0, None, None)?;
    Ok(())
}

fn append_args(command: &mut Command, args: &str) {
    for arg in args.split_whitespace() {
        command.arg(arg);
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

fn run_windows_command(program: &str, args: &[&str]) -> Result<(), Box<dyn Error>> {
    let status = Command::new(program).args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} failed with status {}", program, status).into())
    }
}
