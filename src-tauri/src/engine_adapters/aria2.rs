use std::{
    error::Error,
    fs,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use base64::{engine::general_purpose, Engine as _};
use reqwest::{blocking::Client, Url};
use serde_json::{json, Value};

use crate::{
    logger,
    models::{DownloadStatus, DownloadTask, EngineSettings, SourceType},
    torrent_metadata::TorrentFileEntry,
};

use super::{
    append_args, bool_param, engine_proxy_url, engine_speed_limit_bytes_per_sec, engine_user_agent,
    log_command, parse_i64, required_engine_task_id, EngineTaskState, MagnetMetadata,
};

const ARIA2_BASE_DEFAULT_ARGS: &str = "--continue=true";
const MAGNET_NAME_RESOLVE_ATTEMPTS: usize = 60;
const COLD_ARIA2_MAGNET_RESOLVE_ATTEMPTS: usize = 180;
const MAGNET_NAME_RESOLVE_INTERVAL: Duration = Duration::from_secs(1);
const ARIA2_STARTUP_ATTEMPTS: usize = 20;
const ARIA2_STARTUP_INTERVAL: Duration = Duration::from_millis(250);
const MAGNET_FALLBACK_TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://tracker.torrent.eu.org:451/announce",
    "udp://tracker.openbittorrent.com:6969/announce",
    "udp://exodus.desync.com:6969/announce",
];

pub(super) fn add_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    add_aria2_task(settings, task)
}

pub(super) fn resolve_magnet_metadata(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<MagnetMetadata, Box<dyn Error>> {
    resolve_aria2_magnet_metadata(settings, source, save_path)
}

pub(super) fn task_torrent_files(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    aria2_file_entries(
        settings,
        required_engine_task_id(task)?,
        Some(&task.save_path),
    )
}

pub(super) fn update_task_file_selection(
    settings: &EngineSettings,
    task: &DownloadTask,
    selected_file_indexes: &[i64],
) -> Result<(), Box<dyn Error>> {
    aria2_rpc(
        settings,
        "aria2.changeOption",
        json!([
            required_engine_task_id(task)?,
            { "select-file": format_select_file_indexes(selected_file_indexes) }
        ]),
    )
    .map(|_| ())
}

pub(super) fn resolve_magnet_files(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    resolve_aria2_magnet_files(settings, source, save_path)
}

pub(super) fn test_connection(settings: &EngineSettings) -> Result<(), Box<dyn Error>> {
    aria2_rpc(settings, "aria2.getVersion", json!([]))?;
    Ok(())
}

pub(super) fn pause_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
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
                file_name: None,
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
        file_name: None,
        error_message: None,
    })
}

pub(super) fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
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
                file_name: None,
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

pub(super) fn delete_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<(), Box<dyn Error>> {
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
    Ok(())
}

pub(super) fn refresh_task(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<EngineTaskState, Box<dyn Error>> {
    refresh_aria2_task(settings, task)
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
        engine_proxy_url(settings),
        engine_user_agent(settings),
        engine_speed_limit_bytes_per_sec(settings),
        settings.aria2_bt_max_peers,
        settings.aria2_max_connection_per_server,
        settings.aria2_split,
        &settings.aria2_min_split_size,
        &settings.aria2_file_allocation,
        settings.aria2_seed_time,
        settings.aria2_seed_ratio,
        settings.aria2_enable_dht,
        settings.aria2_enable_dht6,
        settings.aria2_enable_peer_exchange,
        settings.aria2_enable_lpd,
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
        SourceType::Magnet => {
            let source = magnet_with_fallback_trackers(settings, &task.source);
            aria2_rpc(settings, "aria2.addUri", json!([[source], options]))?
        }
        _ => aria2_rpc(settings, "aria2.addUri", json!([[task.source], options]))?,
    };

    let gid = result
        .as_str()
        .ok_or("aria2 did not return a task gid")?
        .to_string();
    Ok(EngineTaskState::running(gid))
}

fn resolve_aria2_magnet_metadata(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<MagnetMetadata, Box<dyn Error>> {
    let started = start_aria2_process(settings, save_path)?;

    let options = aria2_download_options(
        save_path,
        None,
        &settings.default_args,
        "",
        None,
        engine_proxy_url(settings),
        engine_user_agent(settings),
        engine_speed_limit_bytes_per_sec(settings),
        settings.aria2_bt_max_peers,
        settings.aria2_max_connection_per_server,
        settings.aria2_split,
        &settings.aria2_min_split_size,
        &settings.aria2_file_allocation,
        settings.aria2_seed_time,
        settings.aria2_seed_ratio,
        settings.aria2_enable_dht,
        settings.aria2_enable_dht6,
        settings.aria2_enable_peer_exchange,
        settings.aria2_enable_lpd,
    );
    let source = magnet_with_fallback_trackers(settings, source);
    let result = aria2_rpc(settings, "aria2.addUri", json!([[source], options]))?;
    let gid = result
        .as_str()
        .ok_or("aria2 did not return a task gid")?
        .to_string();
    let attempts = if started {
        COLD_ARIA2_MAGNET_RESOLVE_ATTEMPTS
    } else {
        MAGNET_NAME_RESOLVE_ATTEMPTS
    };
    let (metadata, cleanup_gids) = poll_aria2_magnet_metadata(settings, &gid, save_path, attempts)?;
    cleanup_aria2_metadata_tasks(settings, &cleanup_gids);
    Ok(metadata)
}

fn resolve_aria2_magnet_files(
    settings: &EngineSettings,
    source: &str,
    save_path: &str,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    let started = start_aria2_process(settings, save_path)?;

    let options = aria2_download_options(
        save_path,
        None,
        &settings.default_args,
        "",
        None,
        engine_proxy_url(settings),
        engine_user_agent(settings),
        engine_speed_limit_bytes_per_sec(settings),
        settings.aria2_bt_max_peers,
        settings.aria2_max_connection_per_server,
        settings.aria2_split,
        &settings.aria2_min_split_size,
        &settings.aria2_file_allocation,
        settings.aria2_seed_time,
        settings.aria2_seed_ratio,
        settings.aria2_enable_dht,
        settings.aria2_enable_dht6,
        settings.aria2_enable_peer_exchange,
        settings.aria2_enable_lpd,
    );
    let source = magnet_with_fallback_trackers(settings, source);
    let result = aria2_rpc(settings, "aria2.addUri", json!([[source], options]))?;
    let gid = result
        .as_str()
        .ok_or("aria2 did not return a task gid")?
        .to_string();
    let attempts = if started {
        COLD_ARIA2_MAGNET_RESOLVE_ATTEMPTS
    } else {
        MAGNET_NAME_RESOLVE_ATTEMPTS
    };
    let (files, cleanup_gids) = poll_aria2_magnet_files(settings, &gid, save_path, attempts)?;
    cleanup_aria2_metadata_tasks(settings, &cleanup_gids);
    Ok(files)
}

fn poll_aria2_magnet_files(
    settings: &EngineSettings,
    gid: &str,
    save_path: &str,
    attempts: usize,
) -> Result<(Option<Vec<TorrentFileEntry>>, Vec<String>), Box<dyn Error>> {
    let fields = ["gid", "status", "bittorrent", "errorMessage", "followedBy"];
    let mut last_error = None;
    let mut cleanup_gids = vec![gid.to_string()];

    for _ in 0..attempts {
        let status = aria2_rpc(settings, "aria2.tellStatus", json!([gid, fields]))?;
        for file_gid in aria2_candidate_file_gids(&status, gid) {
            let files = aria2_file_entries(settings, &file_gid, Some(save_path))?;
            if !files.is_empty() {
                if file_gid != gid {
                    cleanup_gids.push(file_gid);
                }
                return Ok((Some(files), cleanup_gids));
            }
        }

        if let Some(message) = status
            .get("errorMessage")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            last_error = Some(message.to_string());
        }

        thread::sleep(MAGNET_NAME_RESOLVE_INTERVAL);
    }

    if let Some(error) = last_error {
        logger::warn(format!("magnet metadata files were not resolved: {error}"));
    }
    Ok((None, cleanup_gids))
}

fn aria2_candidate_file_gids(status: &Value, gid: &str) -> Vec<String> {
    let mut gids = status
        .get("followedBy")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if aria2_bittorrent_name(status).is_some() {
        gids.push(gid.to_string());
    }
    gids
}

fn aria2_file_entries(
    settings: &EngineSettings,
    gid: &str,
    save_path: Option<&str>,
) -> Result<Vec<TorrentFileEntry>, Box<dyn Error>> {
    let files = aria2_rpc(settings, "aria2.getFiles", json!([gid]))?;
    let Some(files) = files.as_array() else {
        return Ok(Vec::new());
    };

    files
        .iter()
        .enumerate()
        .filter_map(|(index, file)| {
            let path = file.get("path")?.as_str()?.trim();
            if path.is_empty() {
                return None;
            }
            Some((index, file, aria2_display_file_path(path, save_path)))
        })
        .map(|(index, file, path)| {
            let length = file
                .get("length")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .ok_or("aria2 file length is missing")?;
            let completed_length = file
                .get("completedLength")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
            Ok(TorrentFileEntry {
                index: i64::try_from(index + 1)?,
                path,
                length,
                completed_length,
            })
        })
        .collect()
}

fn aria2_display_file_path(path: &str, save_path: Option<&str>) -> String {
    let Some(save_path) = save_path.map(str::trim).filter(|value| !value.is_empty()) else {
        return path.to_string();
    };

    if let Some(display_path) = strip_path_prefix(path, save_path) {
        return display_path;
    }

    path.to_string()
}

fn strip_path_prefix(path: &str, prefix: &str) -> Option<String> {
    let path_parts = split_path_parts(path).collect::<Vec<_>>();
    let prefix_parts = split_path_parts(prefix).collect::<Vec<_>>();
    if prefix_parts.is_empty() || path_parts.len() <= prefix_parts.len() {
        return None;
    }

    let matches = path_parts
        .iter()
        .zip(prefix_parts.iter())
        .all(|(path_part, prefix_part)| {
            if cfg!(windows) {
                path_part.eq_ignore_ascii_case(prefix_part)
            } else {
                path_part == prefix_part
            }
        });
    if !matches {
        return None;
    }

    Some(path_parts[prefix_parts.len()..].join("/"))
}

fn poll_aria2_magnet_metadata(
    settings: &EngineSettings,
    gid: &str,
    save_path: &str,
    attempts: usize,
) -> Result<(MagnetMetadata, Vec<String>), Box<dyn Error>> {
    let fields = ["gid", "status", "bittorrent", "errorMessage", "followedBy"];
    let mut last_error = None;
    let mut cleanup_gids = vec![gid.to_string()];

    for _ in 0..attempts {
        let status = aria2_rpc(settings, "aria2.tellStatus", json!([gid, fields]))?;
        if let Some(name) = aria2_bittorrent_name(&status) {
            let files = aria2_files_from_candidate_gids(
                settings,
                &status,
                gid,
                save_path,
                &mut cleanup_gids,
            )?;
            return Ok((
                MagnetMetadata {
                    name: Some(name.to_string()),
                    files,
                },
                cleanup_gids,
            ));
        }
        if let Some((next_gid, name)) = status
            .get("followedBy")
            .and_then(Value::as_array)
            .and_then(|gids| gids.first())
            .and_then(Value::as_str)
            .and_then(|next_gid| {
                aria2_rpc(settings, "aria2.tellStatus", json!([next_gid, fields]))
                    .ok()
                    .and_then(|next_status| {
                        aria2_bittorrent_name(&next_status)
                            .map(ToOwned::to_owned)
                            .or_else(|| {
                                aria2_task_top_level_name(settings, next_gid).ok().flatten()
                            })
                            .map(|name| (next_gid.to_string(), name.to_string()))
                    })
            })
        {
            let next_status =
                aria2_rpc(settings, "aria2.tellStatus", json!([next_gid, fields])).ok();
            let files = next_status
                .as_ref()
                .and_then(|status| {
                    aria2_files_from_candidate_gids(
                        settings,
                        status,
                        &next_gid,
                        save_path,
                        &mut cleanup_gids,
                    )
                    .ok()
                    .flatten()
                })
                .or_else(|| {
                    aria2_file_entries(settings, &next_gid, Some(save_path))
                        .ok()
                        .filter(|files| !files.is_empty())
                });
            if !cleanup_gids.contains(&next_gid) {
                cleanup_gids.push(next_gid);
            }
            return Ok((
                MagnetMetadata {
                    name: Some(name),
                    files,
                },
                cleanup_gids,
            ));
        }

        if let Some(message) = status
            .get("errorMessage")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            last_error = Some(message.to_string());
        }

        thread::sleep(MAGNET_NAME_RESOLVE_INTERVAL);
    }

    if let Some(error) = last_error {
        logger::warn(format!("magnet metadata name was not resolved: {error}"));
    }
    Ok((
        MagnetMetadata {
            name: None,
            files: None,
        },
        cleanup_gids,
    ))
}

fn aria2_files_from_candidate_gids(
    settings: &EngineSettings,
    status: &Value,
    gid: &str,
    save_path: &str,
    cleanup_gids: &mut Vec<String>,
) -> Result<Option<Vec<TorrentFileEntry>>, Box<dyn Error>> {
    for file_gid in aria2_candidate_file_gids(status, gid) {
        let files = aria2_file_entries(settings, &file_gid, Some(save_path))?;
        if !files.is_empty() {
            if file_gid != gid && !cleanup_gids.contains(&file_gid) {
                cleanup_gids.push(file_gid);
            }
            return Ok(Some(files));
        }
    }
    Ok(None)
}

fn aria2_bittorrent_name(status: &Value) -> Option<&str> {
    status
        .get("bittorrent")?
        .get("info")?
        .get("name")?
        .as_str()
        .filter(|value| !value.trim().is_empty())
}

fn aria2_task_top_level_name(
    settings: &EngineSettings,
    gid: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    let files = aria2_rpc(settings, "aria2.getFiles", json!([gid]))?;
    let Some(files) = files.as_array() else {
        return Ok(None);
    };

    let paths = files
        .iter()
        .filter_map(|file| file.get("path").and_then(Value::as_str))
        .filter(|path| !path.trim().is_empty())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(None);
    }

    Ok(common_download_entry_name(&paths).map(ToOwned::to_owned))
}

fn common_download_entry_name<'a>(paths: &'a [&'a str]) -> Option<&'a str> {
    let first_parts = split_path_parts(paths.first()?).collect::<Vec<_>>();
    if first_parts.is_empty() {
        return None;
    }

    let common_len = paths
        .iter()
        .skip(1)
        .fold(first_parts.len(), |common, path| {
            let parts = split_path_parts(path).collect::<Vec<_>>();
            first_parts
                .iter()
                .zip(parts.iter())
                .take(common)
                .take_while(|(left, right)| left == right)
                .count()
        });

    if common_len == 0 {
        return first_parts.first().copied();
    }

    first_parts.get(common_len.saturating_sub(1)).copied()
}

fn split_path_parts(path: &str) -> impl Iterator<Item = &str> {
    path.split(['/', '\\'])
        .filter(|part| !part.trim().is_empty())
}

fn magnet_with_fallback_trackers(settings: &EngineSettings, source: &str) -> String {
    if !source
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("magnet:")
    {
        return source.to_string();
    }

    let trackers = settings
        .trackers
        .iter()
        .map(String::as_str)
        .chain(MAGNET_FALLBACK_TRACKERS.iter().copied());

    trackers.fold(source.to_string(), |mut value, tracker| {
        if !magnet_has_tracker(&value, tracker) {
            value.push_str("&tr=");
            value.push_str(&percent_encode_query_value(tracker));
        }
        value
    })
}

fn magnet_has_tracker(source: &str, tracker: &str) -> bool {
    source.split('&').any(|part| {
        part.strip_prefix("tr=")
            .and_then(|value| percent_decode_query_value(value).ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(tracker))
    })
}

fn percent_encode_query_value(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                vec![byte as char]
            }
            _ => format!("%{byte:02X}").chars().collect(),
        })
        .collect()
}

fn percent_decode_query_value(value: &str) -> Result<String, Box<dyn Error>> {
    let mut bytes = Vec::new();
    let mut input = value.as_bytes().iter().copied();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input.next().ok_or("invalid percent encoding")?;
            let low = input.next().ok_or("invalid percent encoding")?;
            bytes.push(hex_pair(high, low)?);
        } else if byte == b'+' {
            bytes.push(b' ');
        } else {
            bytes.push(byte);
        }
    }
    Ok(String::from_utf8(bytes)?)
}

fn hex_pair(high: u8, low: u8) -> Result<u8, Box<dyn Error>> {
    Ok(hex_value(high)? * 16 + hex_value(low)?)
}

fn hex_value(value: u8) -> Result<u8, Box<dyn Error>> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err("invalid percent encoding".into()),
    }
}

fn cleanup_aria2_metadata_tasks(settings: &EngineSettings, gids: &[String]) {
    for gid in gids.iter().rev() {
        let _ = aria2_rpc(settings, "aria2.remove", json!([gid]));
        let _ = aria2_rpc(settings, "aria2.removeDownloadResult", json!([gid]));
    }
}

fn start_aria2_process(settings: &EngineSettings, save_path: &str) -> Result<bool, Box<dyn Error>> {
    if aria2_rpc(settings, "aria2.getVersion", json!([])).is_ok() {
        return Ok(false);
    }

    let executable = settings.executable_path.as_deref().unwrap_or("").trim();
    if executable.is_empty() {
        return Ok(false);
    }

    let rpc_listen_port = aria2_rpc_listen_port(settings)?;
    let mut command = Command::new(executable);
    command
        .arg("--enable-rpc=true")
        .arg("--rpc-listen-all=false")
        .args(ARIA2_BASE_DEFAULT_ARGS.split_whitespace());
    append_args(&mut command, &settings.default_args);
    command
        .arg(format!(
            "--max-connection-per-server={}",
            settings.aria2_max_connection_per_server
        ))
        .arg(format!("--split={}", settings.aria2_split))
        .arg(format!(
            "--min-split-size={}",
            settings.aria2_min_split_size
        ))
        .arg(format!(
            "--file-allocation={}",
            settings.aria2_file_allocation
        ))
        .arg(format!("--rpc-listen-port={rpc_listen_port}"))
        .arg(format!(
            "--enable-dht={}",
            bool_param(settings.aria2_enable_dht)
        ))
        .arg(format!(
            "--enable-dht6={}",
            bool_param(settings.aria2_enable_dht6)
        ))
        .arg(format!(
            "--enable-peer-exchange={}",
            bool_param(settings.aria2_enable_peer_exchange)
        ))
        .arg(format!(
            "--bt-enable-lpd={}",
            bool_param(settings.aria2_enable_lpd)
        ))
        .arg(format!("--listen-port={}", settings.aria2_bt_listen_port))
        .arg(format!("--bt-max-peers={}", settings.aria2_bt_max_peers))
        .arg(format!("--seed-time={}", settings.aria2_seed_time))
        .arg(format!("--seed-ratio={}", settings.aria2_seed_ratio))
        .arg(format!("--dir={}", save_path))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    log_command("starting aria2", &command);
    command.spawn().map_err(|error| {
        logger::error(format!("aria2 spawn failed: {error}"));
        error
    })?;
    wait_for_aria2_rpc(settings)?;
    Ok(true)
}

fn wait_for_aria2_rpc(settings: &EngineSettings) -> Result<(), Box<dyn Error>> {
    let mut last_error = None;
    for _ in 0..ARIA2_STARTUP_ATTEMPTS {
        match aria2_rpc(settings, "aria2.getVersion", json!([])) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = Some(error.to_string()),
        }
        thread::sleep(ARIA2_STARTUP_INTERVAL);
    }

    Err(format!(
        "aria2 rpc was not ready after startup: {}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    )
    .into())
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
                "followedBy",
                "bittorrent",
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
                file_name: None,
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
    let bittorrent_name = aria2_bittorrent_name(&result)
        .map(str::trim)
        .map(ToOwned::to_owned);
    let followed_by = result
        .get("followedBy")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .next()
        .map(ToOwned::to_owned);
    let next_engine_task_id = followed_by.or_else(|| task.engine_task_id.clone());
    let normalized_status = match status {
        "complete"
            if task.source_type == SourceType::Magnet
                && next_engine_task_id.as_deref() != Some(gid) =>
        {
            DownloadStatus::Running
        }
        "complete" => DownloadStatus::Completed,
        "active" => DownloadStatus::Running,
        "waiting" => DownloadStatus::Queued,
        "paused" => DownloadStatus::Paused,
        "removed" => DownloadStatus::Deleted,
        _ => DownloadStatus::Failed,
    };

    Ok(EngineTaskState {
        status: normalized_status,
        progress,
        speed_bytes_per_sec: speed,
        downloaded_bytes: completed,
        total_bytes: total,
        engine_task_id: next_engine_task_id,
        file_name: bittorrent_name,
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

pub(super) fn aria2_rpc_url(settings: &EngineSettings) -> String {
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

fn aria2_rpc_listen_port(settings: &EngineSettings) -> Result<u16, Box<dyn Error>> {
    let url = Url::parse(&aria2_rpc_url(settings))?;
    url.port()
        .ok_or_else(|| "aria2 rpc url must include port".into())
}

pub(super) fn aria2_params(
    settings: &EngineSettings,
    params: Value,
) -> Result<Value, Box<dyn Error>> {
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

pub(super) fn aria2_download_options(
    save_path: &str,
    output_file_name: Option<&str>,
    default_args: &str,
    task_args: &str,
    selected_file_indexes: Option<&[i64]>,
    proxy_url: Option<&str>,
    user_agent: Option<&str>,
    speed_limit_bytes_per_sec: Option<i64>,
    aria2_bt_max_peers: i64,
    aria2_max_connection_per_server: i64,
    aria2_split: i64,
    aria2_min_split_size: &str,
    aria2_file_allocation: &str,
    aria2_seed_time: i64,
    aria2_seed_ratio: f64,
    aria2_enable_dht: bool,
    aria2_enable_dht6: bool,
    aria2_enable_peer_exchange: bool,
    aria2_enable_lpd: bool,
) -> Value {
    let mut options = serde_json::Map::new();
    options.insert("dir".to_string(), Value::String(save_path.to_string()));
    append_aria2_options(&mut options, ARIA2_BASE_DEFAULT_ARGS);
    append_aria2_options(&mut options, default_args);
    append_aria2_options(&mut options, task_args);
    insert_i64_option(&mut options, "bt-max-peers", aria2_bt_max_peers);
    insert_i64_option(
        &mut options,
        "max-connection-per-server",
        aria2_max_connection_per_server,
    );
    insert_i64_option(&mut options, "split", aria2_split);
    options.insert(
        "min-split-size".to_string(),
        Value::String(aria2_min_split_size.to_string()),
    );
    options.insert(
        "file-allocation".to_string(),
        Value::String(aria2_file_allocation.to_string()),
    );
    insert_i64_option(&mut options, "seed-time", aria2_seed_time);
    insert_f64_option(&mut options, "seed-ratio", aria2_seed_ratio);
    insert_bool_option(&mut options, "enable-dht", aria2_enable_dht);
    insert_bool_option(&mut options, "enable-dht6", aria2_enable_dht6);
    insert_bool_option(
        &mut options,
        "enable-peer-exchange",
        aria2_enable_peer_exchange,
    );
    insert_bool_option(&mut options, "bt-enable-lpd", aria2_enable_lpd);
    if let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) {
        options.insert(
            "all-proxy".to_string(),
            Value::String(proxy_url.to_string()),
        );
    }
    if let Some(user_agent) = user_agent.map(str::trim).filter(|value| !value.is_empty()) {
        options.insert(
            "user-agent".to_string(),
            Value::String(user_agent.to_string()),
        );
    }
    if let Some(speed_limit_bytes_per_sec) = speed_limit_bytes_per_sec.filter(|value| *value > 0) {
        insert_i64_option(
            &mut options,
            "max-download-limit",
            speed_limit_bytes_per_sec,
        );
    }
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

fn insert_bool_option(options: &mut serde_json::Map<String, Value>, key: &str, value: bool) {
    options.insert(
        key.to_string(),
        Value::String(bool_param(value).to_string()),
    );
}

fn insert_i64_option(options: &mut serde_json::Map<String, Value>, key: &str, value: i64) {
    options.insert(key.to_string(), Value::String(value.to_string()));
}

fn insert_f64_option(options: &mut serde_json::Map<String, Value>, key: &str, value: f64) {
    options.insert(key.to_string(), Value::String(value.to_string()));
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
        "dir" | "enable-rpc" | "listen-port" | "rpc-listen-all" | "rpc-listen-port" | "rpc-secret"
    )
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
