use std::{error::Error, fs, path::Path, thread, time::Duration};

use reqwest::blocking::{multipart, Client};
use serde::Deserialize;

use crate::{
    logger,
    models::{DownloadStatus, DownloadTask, EngineSettings, RemoteDirectoryEntry, SourceType},
    torrent_metadata::{read_torrent_info_hash, TorrentFileEntry},
};

use super::{
    bool_param, parse_magnet_hash, required_engine_task_id, EngineTaskState, MagnetMetadata,
};

const MAGNET_NAME_RESOLVE_ATTEMPTS: usize = 60;
const MAGNET_NAME_RESOLVE_INTERVAL: Duration = Duration::from_secs(1);

pub(super) fn add_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    add_qbittorrent_task(settings, task)
}

pub(super) fn resolve_magnet_metadata(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<MagnetMetadata, Box<dyn Error>> {
    Ok(MagnetMetadata {
        name: resolve_qbittorrent_magnet_name(settings, source, save_path)?,
        files: None,
    })
}

pub(super) fn task_torrent_files(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    qbittorrent_task_files(settings, task)
}

pub(super) fn update_task_file_selection(
    settings: &EngineSettings,
    task: &DownloadTask,
    selected_file_indexes: &[i64],
) -> Result<(), Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    qbittorrent_select_files(
        &client,
        settings,
        required_engine_task_id(task)?,
        selected_file_indexes,
    )
}

pub(super) fn resolve_magnet_files(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    resolve_qbittorrent_magnet_files(settings, source, save_path)
}

pub(super) fn list_remote_directories(
    settings: &EngineSettings,
    path: &str,
) -> Result<Vec<RemoteDirectoryEntry>, Box<dyn Error>> {
    list_qbittorrent_directories(settings, path)
}

pub(super) fn test_connection(settings: &EngineSettings) -> Result<(), Box<dyn Error>> {
    qbittorrent_client(settings)?;
    Ok(())
}

pub(super) fn pause_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
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
        file_name: None,
        error_message: None,
    })
}

pub(super) fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
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
        file_name: None,
        error_message: None,
    })
}

pub(super) fn delete_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    delete_files: bool,
) -> Result<(), Box<dyn Error>> {
    let hash = required_engine_task_id(task)?;
    let client = qbittorrent_client(settings)?;
    qbittorrent_post(
        &client,
        settings,
        "torrents/delete",
        &[("hashes", hash), ("deleteFiles", bool_param(delete_files))],
    )
}

pub(super) fn refresh_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    refresh_qbittorrent_task(settings, task)
}
fn add_qbittorrent_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let mut form = multipart::Form::new().text(
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
    let save_path = task.save_path.trim();
    if !save_path.is_empty() {
        form = form.text("savepath", save_path.to_string());
    }
    form = apply_qbittorrent_task_options(settings, form);

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

fn apply_qbittorrent_task_options(
    settings: &EngineSettings,
    mut form: multipart::Form,
) -> multipart::Form {
    if settings.qbittorrent_download_limit_bytes_per_sec > 0 {
        form = form.text(
            "dlLimit",
            settings
                .qbittorrent_download_limit_bytes_per_sec
                .to_string(),
        );
    }
    if settings.qbittorrent_upload_limit_bytes_per_sec > 0 {
        form = form.text(
            "upLimit",
            settings.qbittorrent_upload_limit_bytes_per_sec.to_string(),
        );
    }
    if settings.qbittorrent_seed_ratio_limit > 0.0 {
        form = form.text(
            "ratioLimit",
            settings.qbittorrent_seed_ratio_limit.to_string(),
        );
    }
    if settings.qbittorrent_seed_time_limit_minutes > 0 {
        form = form.text(
            "seedingTimeLimit",
            settings.qbittorrent_seed_time_limit_minutes.to_string(),
        );
    }
    form
}

fn resolve_qbittorrent_magnet_name(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let hash = parse_magnet_hash(source).ok_or("magnet hash is required")?;
    let existed = !qbittorrent_get_torrents(&client, settings, &hash)?.is_empty();

    if !existed {
        let mut form = multipart::Form::new()
            .text("paused", "false")
            .text("urls", source.to_string());
        let save_path = save_path.trim();
        if !save_path.is_empty() {
            form = form.text("savepath", save_path.to_string());
        }

        let response = client
            .post(qbittorrent_url(settings, "torrents/add")?)
            .multipart(form)
            .send()?;
        if !response.status().is_success() {
            return Err(format!("qBittorrent add failed: {}", response.status()).into());
        }
    }

    let resolved = poll_qbittorrent_magnet_name(&client, settings, &hash);
    if !existed {
        cleanup_qbittorrent_metadata_task(&client, settings, &hash);
    }
    resolved
}

fn resolve_qbittorrent_magnet_files(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let hash = parse_magnet_hash(source).ok_or("magnet hash is required")?;
    let existed = !qbittorrent_get_torrents(&client, settings, &hash)?.is_empty();

    if !existed {
        let mut form = multipart::Form::new()
            .text("paused", "false")
            .text("urls", source.to_string());
        let save_path = save_path.trim();
        if !save_path.is_empty() {
            form = form.text("savepath", save_path.to_string());
        }

        let response = client
            .post(qbittorrent_url(settings, "torrents/add")?)
            .multipart(form)
            .send()?;
        if !response.status().is_success() {
            return Err(format!("qBittorrent add failed: {}", response.status()).into());
        }
    }

    let resolved = poll_qbittorrent_magnet_files(&client, settings, &hash);
    if !existed {
        cleanup_qbittorrent_metadata_task(&client, settings, &hash);
    }
    resolved
}

fn poll_qbittorrent_magnet_files(
    client: &Client,
    settings: &EngineSettings,
    hash: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    let mut last_state = None;

    for _ in 0..MAGNET_NAME_RESOLVE_ATTEMPTS {
        let torrents = qbittorrent_get_torrents(client, settings, hash)?;
        if let Some(torrent) = torrents.first() {
            last_state = Some(torrent.state.clone());
            let name = torrent.name.trim();
            if name.is_empty() || name.eq_ignore_ascii_case(hash) {
                thread::sleep(MAGNET_NAME_RESOLVE_INTERVAL);
                continue;
            }

            let files = qbittorrent_get_files(client, settings, hash)?;
            if !files.is_empty() {
                return files
                    .into_iter()
                    .map(|file| {
                        let completed_length = file.progress_bytes();
                        Ok(TorrentFileEntry {
                            index: file.index + 1,
                            path: file.name,
                            length: file.size,
                            completed_length,
                        })
                    })
                    .collect::<Result<Vec<_>, Box<dyn Error>>>()
                    .map(Some);
            }
        }

        thread::sleep(MAGNET_NAME_RESOLVE_INTERVAL);
    }

    if let Some(state) = last_state {
        logger::warn(format!("magnet metadata files were not resolved: {state}"));
    } else {
        logger::warn(format!(
            "qBittorrent task not found while resolving magnet files: {hash}"
        ));
    }
    Ok(None)
}

fn poll_qbittorrent_magnet_name(
    client: &Client,
    settings: &EngineSettings,
    hash: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut last_state = None;

    for _ in 0..MAGNET_NAME_RESOLVE_ATTEMPTS {
        let torrents = qbittorrent_get_torrents(client, settings, hash)?;
        if let Some(torrent) = torrents.first() {
            let name = torrent.name.trim();
            if !name.is_empty() && !name.eq_ignore_ascii_case(hash) {
                return Ok(Some(name.to_string()));
            }
            last_state = Some(torrent.state.clone());
        }

        thread::sleep(MAGNET_NAME_RESOLVE_INTERVAL);
    }

    if let Some(state) = last_state {
        logger::warn(format!("magnet metadata name was not resolved: {state}"));
    } else {
        logger::warn(format!(
            "qBittorrent task not found while resolving magnet name: {hash}"
        ));
    }
    Ok(None)
}

fn cleanup_qbittorrent_metadata_task(client: &Client, settings: &EngineSettings, hash: &str) {
    let _ = qbittorrent_post(
        client,
        settings,
        "torrents/delete",
        &[("hashes", hash), ("deleteFiles", "false")],
    );
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
        file_name: Some(torrent.name.clone()),
        error_message: None,
    })
}

#[derive(Deserialize)]
struct QbTorrentInfo {
    name: String,
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

fn list_qbittorrent_directories(
    settings: &EngineSettings,
    path: &str,
) -> Result<Vec<RemoteDirectoryEntry>, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    let path = path.trim();
    if path.is_empty() {
        return Err("remote path is required".into());
    }
    let response = client
        .get(qbittorrent_url(settings, "app/getDirectoryContent")?)
        .query(&[
            ("dirPath", path),
            ("mode", "dirs"),
            ("withMetadata", "false"),
        ])
        .send()?;
    if !response.status().is_success() {
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err("qBittorrent remote directory browsing requires a Web API version that supports app/getDirectoryContent".into());
        }
        return Err(format!(
            "qBittorrent directory listing failed: {}",
            response.status()
        )
        .into());
    }

    let mut entries = response
        .json::<Vec<String>>()?
        .into_iter()
        .map(|entry_path| RemoteDirectoryEntry {
            name: remote_path_name(&entry_path),
            path: entry_path,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    Ok(entries)
}

fn remote_path_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn qbittorrent_client(settings: &EngineSettings) -> Result<Client, Box<dyn Error>> {
    let mut builder = Client::builder().cookie_store(true);
    if let Some(proxy_url) = settings
        .proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.proxy(reqwest::Proxy::all(proxy_url)?);
    }
    let client = builder.build()?;
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
    name: String,
    size: i64,
    progress: f64,
}

impl QbTorrentFileInfo {
    fn progress_bytes(&self) -> i64 {
        (self.size as f64 * self.progress).round() as i64
    }
}

fn qbittorrent_task_files(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    let client = qbittorrent_client(settings)?;
    qbittorrent_get_files(&client, settings, required_engine_task_id(task)?)?
        .into_iter()
        .map(|file| {
            let completed_length = file.progress_bytes();
            Ok(TorrentFileEntry {
                index: file.index + 1,
                path: file.name,
                length: file.size,
                completed_length,
            })
        })
        .collect()
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

fn qbittorrent_get_torrents(
    client: &Client,
    settings: &EngineSettings,
    hash: &str,
) -> Result<Vec<QbTorrentInfo>, Box<dyn Error>> {
    let response = client
        .get(qbittorrent_url(settings, "torrents/info")?)
        .query(&[("hashes", hash)])
        .send()?;
    if !response.status().is_success() {
        return Err(format!("qBittorrent status failed: {}", response.status()).into());
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

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    use crate::models::{EngineKind, SourceType};

    use super::*;

    #[test]
    fn qbittorrent_client_routes_login_through_configured_proxy() {
        let (proxy_url, requests, server) = start_fake_http_proxy(1);
        let mut settings = qbittorrent_settings("http://qbittorrent.test");
        settings.proxy_url = Some(proxy_url);

        qbittorrent_client(&settings).expect("login should succeed through proxy");

        server.join().expect("fake proxy should finish");
        let request = requests
            .lock()
            .expect("requests should lock")
            .first()
            .cloned()
            .expect("proxy should receive login request");
        assert!(request.starts_with("POST http://qbittorrent.test/api/v2/auth/login "));
    }

    fn qbittorrent_settings(connection_url: &str) -> EngineSettings {
        EngineSettings {
            id: "qbittorrent".to_string(),
            engine: EngineKind::QBittorrent,
            name: "qBittorrent".to_string(),
            enabled: true,
            executable_path: None,
            default_download_dir: String::new(),
            default_args: String::new(),
            connection_url: Some(connection_url.to_string()),
            username: Some("admin".to_string()),
            password: Some("adminadmin".to_string()),
            remote_path: Some("/downloads".to_string()),
            supported_source_types: vec![SourceType::Magnet, SourceType::Torrent],
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
            aria2_enable_dht: false,
            aria2_enable_dht6: false,
            aria2_enable_peer_exchange: false,
            aria2_enable_lpd: false,
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

    fn start_fake_http_proxy(
        expected_requests: usize,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fake proxy should bind");
        let address = listener
            .local_addr()
            .expect("fake proxy should have address");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = Arc::clone(&requests);
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(expected_requests) {
                let mut stream = stream.expect("fake proxy stream should open");
                let mut buffer = [0_u8; 4096];
                let read = stream.read(&mut buffer).expect("request should read");
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                server_requests
                    .lock()
                    .expect("requests should lock")
                    .push(request);
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nOk.",
                    )
                    .expect("response should write");
            }
        });

        (format!("http://{address}"), requests, server)
    }
}
