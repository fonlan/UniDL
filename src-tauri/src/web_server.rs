use std::{
    collections::HashSet,
    error::Error,
    fs,
    io::{Cursor, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server, StatusCode};
use uuid::Uuid;

use crate::{
    db, engine_install, logger,
    models::{
        AppSettings, AppSettingsInput, CreateDownloadTaskInput, EngineKind, EngineSettings,
        SourceType,
    },
    repositories::EngineSettingsRepository,
    services::DownloadTaskService,
    system_open, task_events,
};

pub struct WebServerHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WebServerHandle {
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for WebServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[derive(Clone)]
struct WebServerContext {
    app_handle: Option<tauri::AppHandle>,
    database_path: PathBuf,
    pending_open_sources: Arc<Mutex<Vec<system_open::OpenTaskRequest>>>,
    web_access_enabled: bool,
    password: String,
    sessions: Arc<Mutex<HashSet<String>>>,
    stop: Arc<AtomicBool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginInput {
    password: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginOutput {
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskIdsInput {
    ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteTasksInput {
    ids: Vec<String>,
    delete_completed_files: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClearDownloadRecordsInput {
    older_than_days: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TorrentFilesInput {
    source: String,
    source_type: SourceType,
    engine_settings_id: Option<String>,
    save_path: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskFileSelectionInput {
    selected_file_indexes: Vec<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackerSubscriptionInput {
    subscription_url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteDirectoriesInput {
    engine_settings_id: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MagnetNameInput {
    source: String,
    engine_settings_id: String,
    save_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidateEngineSourceTypeInput {
    engine: EngineKind,
    source_type: SourceType,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagedEngineExecutablePathInput {
    engine: EngineKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionVideosInput {
    source: String,
    title: Option<String>,
    cookies: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionVideoEntry {
    id: String,
    title: String,
    source: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionVideosOutput {
    videos: Vec<ExtensionVideoEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionVideosSupportOutput {
    can_detect_videos: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionYtDlpTaskInput {
    source: String,
    file_name: String,
    cookies: Option<String>,
    http_referrer: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionTaskInput {
    source_type: SourceType,
    source: String,
    file_name: String,
    http_referrer: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionTaskOutput {
    file_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorOutput {
    error: String,
}

pub fn start(
    app_handle: tauri::AppHandle,
    database_path: PathBuf,
    pending_open_sources: Arc<Mutex<Vec<system_open::OpenTaskRequest>>>,
    settings: &AppSettings,
) -> Result<WebServerHandle, Box<dyn Error + Send + Sync>> {
    let bind_address = bind_address_from_url(&settings.web_access_url)?;
    start_on(
        &bind_address.to_string(),
        Some(app_handle),
        database_path,
        pending_open_sources,
        settings,
    )
}

pub fn bind_address_from_url(url: &str) -> Result<SocketAddr, Box<dyn Error + Send + Sync>> {
    let trimmed = url.trim();
    let address = trimmed
        .strip_prefix("http://")
        .ok_or("web access URL must start with http://")?;
    if address.contains('/') || address.contains('?') || address.contains('#') {
        return Err("web access URL must only include host and port".into());
    }
    address.parse::<SocketAddr>().map_err(|error| error.into())
}

fn start_on(
    bind_address: &str,
    app_handle: Option<tauri::AppHandle>,
    database_path: PathBuf,
    pending_open_sources: Arc<Mutex<Vec<system_open::OpenTaskRequest>>>,
    settings: &AppSettings,
) -> Result<WebServerHandle, Box<dyn Error + Send + Sync>> {
    let server = Server::http(bind_address)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let context = WebServerContext {
        app_handle,
        database_path,
        pending_open_sources,
        web_access_enabled: settings.web_access_enabled,
        password: settings.web_access_password.clone(),
        sessions: Arc::new(Mutex::new(HashSet::new())),
        stop: Arc::clone(&stop),
    };

    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::Relaxed) {
            match server.recv_timeout(Duration::from_millis(250)) {
                Ok(Some(request)) => {
                    let context = context.clone();
                    thread::spawn(move || handle_request(request, &context));
                }
                Ok(None) => {}
                Err(error) => logger::error(format!("web server request error: {error}")),
            }
        }
    });

    Ok(WebServerHandle {
        stop,
        thread: Some(thread),
    })
}

fn handle_request(mut request: Request, context: &WebServerContext) {
    let method = request.method().clone();
    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or(request.url())
        .to_string();

    let result = if method == Method::Options {
        Ok(empty_response(StatusCode(204)))
    } else if method == Method::Get && path == "/api/health" {
        json_response(StatusCode(200), &serde_json::json!({ "ok": true }))
    } else if method == Method::Post && path == "/api/extension/videos" {
        handle_extension_videos(&mut request, context)
    } else if method == Method::Post && path == "/api/extension/videos/support" {
        handle_extension_videos_support(&mut request, context)
    } else if method == Method::Post && path == "/api/extension/ytdlp/tasks" {
        handle_extension_ytdlp_task(&mut request, context)
    } else if method == Method::Post && path == "/api/extension/tasks" {
        handle_extension_task(&mut request, context)
    } else if !context.web_access_enabled {
        json_response(
            StatusCode(403),
            &ErrorOutput {
                error: "web access is disabled".to_string(),
            },
        )
    } else if method == Method::Get && !path.starts_with("/api/") {
        serve_frontend_asset(context, &path)
    } else if method == Method::Post && path == "/api/login" {
        handle_login(&mut request, context)
    } else if !is_authorized(&request, context) {
        json_response(
            StatusCode(401),
            &ErrorOutput {
                error: "unauthorized".to_string(),
            },
        )
    } else {
        handle_authorized_request(&mut request, context, &method, &path)
    };

    let response = match result {
        Ok(response) => response,
        Err(error) => {
            logger::error(format!("web request failed: {method} {path}: {error}"));
            json_response(
                StatusCode(500),
                &ErrorOutput {
                    error: error.to_string(),
                },
            )
            .expect("failed to build error response")
        }
    };

    let _ = request.respond(response);
}

fn handle_login(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: LoginInput = read_json(request)?;
    if input.password != context.password {
        return json_response(
            StatusCode(401),
            &ErrorOutput {
                error: "invalid password".to_string(),
            },
        );
    }

    let token = Uuid::new_v4().to_string();
    context
        .sessions
        .lock()
        .map_err(|_| "web session lock was poisoned")?
        .insert(token.clone());

    json_response(StatusCode(200), &LoginOutput { token })
}

fn handle_extension_videos_support(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionVideosInput = read_json(request)?;
    let source = input.source.trim();
    let can_detect_videos = if let Some(source_type) = extension_source_type(source) {
        find_enabled_ytdlp_settings(context, source_type)?
            .as_ref()
            .and_then(ytdlp_executable_path)
            .is_some()
    } else {
        false
    };

    json_response(
        StatusCode(200),
        &ExtensionVideosSupportOutput { can_detect_videos },
    )
}

fn handle_extension_videos(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionVideosInput = read_json(request)?;
    let source = input.source.trim();
    let Some(source_type) = extension_source_type(source) else {
        return json_response(
            StatusCode(200),
            &ExtensionVideosOutput { videos: Vec::new() },
        );
    };

    let Some(settings) = find_enabled_ytdlp_settings(context, source_type)? else {
        return json_response(
            StatusCode(200),
            &ExtensionVideosOutput { videos: Vec::new() },
        );
    };

    let Some(executable_path) = ytdlp_executable_path(&settings) else {
        return json_response(
            StatusCode(200),
            &ExtensionVideosOutput { videos: Vec::new() },
        );
    };

    let proxy_url = engine_proxy_url(&settings);
    let cookie_path = write_detection_cookie_file(input.cookies.as_deref())?;

    let result = extract_extension_videos(
        executable_path,
        source,
        input.title.as_deref(),
        proxy_url,
        cookie_path.as_deref(),
    );

    if let Some(cookie_path) = &cookie_path {
        let _ = fs::remove_file(cookie_path);
    }

    let videos = result?;

    json_response(StatusCode(200), &ExtensionVideosOutput { videos })
}

fn handle_extension_ytdlp_task(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionYtDlpTaskInput = read_json(request)?;
    let source = input.source.trim().to_string();
    let file_name = input.file_name.trim().to_string();
    let source_type =
        extension_source_type(&source).ok_or("yt-dlp source must be http or https")?;
    find_enabled_ytdlp_settings(context, source_type)?
        .ok_or("no enabled yt-dlp settings supports this page")?;

    open_task_dialog(
        context,
        system_open::OpenTaskRequest {
            source,
            file_name: Some(file_name.clone()),
            browser_cookies: input.cookies.filter(|value| !value.trim().is_empty()),
            http_referrer: normalize_optional_http_url(input.http_referrer),
        },
    )?;
    json_response(StatusCode(200), &ExtensionTaskOutput { file_name })
}

fn ytdlp_executable_path(settings: &EngineSettings) -> Option<&str> {
    settings
        .executable_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn find_enabled_ytdlp_settings(
    context: &WebServerContext,
    source_type: SourceType,
) -> Result<Option<EngineSettings>, Box<dyn Error>> {
    let connection = db::connect_path(context.database_path.clone())?;
    Ok(EngineSettingsRepository::new(&connection)
        .list_all()?
        .into_iter()
        .find(|settings| {
            settings.engine == EngineKind::YtDlp
                && settings.enabled
                && settings.supported_source_types.contains(&source_type)
        }))
}

fn extract_extension_videos(
    executable_path: &str,
    source: &str,
    page_title: Option<&str>,
    proxy_url: Option<&str>,
    cookie_path: Option<&Path>,
) -> Result<Vec<ExtensionVideoEntry>, Box<dyn Error>> {
    let mut command = Command::new(executable_path);
    crate::engine_adapters::apply_ytdlp_utf8_env(&mut command);
    command.args([
        "--dump-single-json",
        "--flat-playlist",
        "--no-playlist",
        "--skip-download",
        "--no-warnings",
        // Detection only needs metadata (title/id/url) — we don't actually pick
        // a format here. Without this flag yt-dlp aborts when the (possibly
        // user-config-imposed) format selector matches nothing, or when YouTube
        // briefly returns no formats due to anti-bot / region / PO Token gating,
        // even though the page itself is perfectly downloadable later.
        "--ignore-no-formats-error",
        // Skip HEAD probes on format URLs — irrelevant for metadata-only runs
        // and a frequent cause of slow / flaky detection.
        "--no-check-formats",
    ]);
    if let Some(proxy_url) = proxy_url {
        command.arg("--proxy").arg(proxy_url);
    }
    if let Some(cookie_path) = cookie_path {
        command.arg("--cookies").arg(cookie_path);
    }
    command.arg(source);

    let output = command.output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stderr_summary = if stderr.is_empty() {
            "(no stderr output)".to_string()
        } else {
            stderr.lines().last().unwrap_or(&stderr).to_string()
        };
        logger::error(format!(
            "yt-dlp video detection failed: source={source}, status={}, stderr={stderr}",
            output.status
        ));
        return Err(format!("yt-dlp 检测失败 (exit {}): {stderr_summary}", output.status).into());
    }

    let value: Value = serde_json::from_slice(&output.stdout)?;
    let videos = if let Some(entries) = value.get("entries").and_then(Value::as_array) {
        entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                if entry.is_null() {
                    None
                } else {
                    Some(build_extension_video_entry(
                        entry, source, index, page_title,
                    ))
                }
            })
            .collect()
    } else {
        vec![build_extension_video_entry(&value, source, 0, page_title)]
    };

    Ok(videos)
}

fn engine_proxy_url(settings: &EngineSettings) -> Option<&str> {
    settings
        .proxy_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn write_detection_cookie_file(cookies: Option<&str>) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let Some(cookies) = cookies else {
        return Ok(None);
    };
    if cookies.trim().is_empty() {
        return Ok(None);
    }
    let cookie_path =
        std::env::temp_dir().join(format!("unidl-detect-{}.cookies.txt", Uuid::new_v4()));
    fs::write(&cookie_path, cookies)?;
    Ok(Some(cookie_path))
}

fn build_extension_video_entry(
    value: &Value,
    source: &str,
    index: usize,
    page_title: Option<&str>,
) -> ExtensionVideoEntry {
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            page_title
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| extension_video_title(source));
    let source = value
        .get("webpage_url")
        .and_then(Value::as_str)
        .or_else(|| value.get("url").and_then(Value::as_str))
        .unwrap_or(source)
        .to_string();
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("current-page-{}", index + 1));

    ExtensionVideoEntry { id, title, source }
}

fn extension_source_type(source: &str) -> Option<SourceType> {
    if source.starts_with("http://") || source.starts_with("https://") {
        Some(SourceType::Http)
    } else {
        None
    }
}

fn normalize_optional_http_url(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            Some(trimmed.to_string())
        } else {
            None
        }
    })
}

fn extension_video_title(source: &str) -> String {
    source
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(|value| {
            value == char::from(47) || value == char::from(63) || value == char::from(35)
        })
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("current page")
        .to_string()
}

fn handle_extension_task(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionTaskInput = read_json(request)?;
    let source = input.source.trim().to_string();
    let file_name = input.file_name.trim().to_string();
    let _ = input.source_type;

    if source.is_empty() {
        return Err("download source is required".into());
    }
    if file_name.is_empty() {
        return Err("file name is required".into());
    }

    let request = system_open::OpenTaskRequest {
        source,
        file_name: Some(file_name.clone()),
        browser_cookies: None,
        http_referrer: normalize_optional_http_url(input.http_referrer),
    };
    open_task_dialog(context, request)?;

    json_response(StatusCode(200), &ExtensionTaskOutput { file_name })
}

fn open_task_dialog(
    context: &WebServerContext,
    request: system_open::OpenTaskRequest,
) -> Result<(), Box<dyn Error>> {
    let requests = vec![request];
    context
        .pending_open_sources
        .lock()
        .map_err(|_| "system open request lock was poisoned")?
        .extend(requests.clone());

    if let Some(app_handle) = &context.app_handle {
        system_open::emit_open_requests(app_handle, requests)?;
        if let Some(window) = app_handle.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
    }

    Ok(())
}

fn handle_authorized_request(
    request: &mut Request,
    context: &WebServerContext,
    method: &Method,
    path: &str,
) -> Result<ResponseBox, Box<dyn Error>> {
    match (method, path) {
        (&Method::Get, "/api/events") => Ok(event_stream_response(context)),
        (&Method::Get, "/api/tasks") => with_task_service(context, |service| {
            json_response(StatusCode(200), &service.list_created_desc()?)
        }),
        (&Method::Post, "/api/tasks") => {
            let input: CreateDownloadTaskInput = read_json(request)?;
            with_task_service(context, |service| {
                json_response(StatusCode(200), &service.create(input)?)
            })
        }
        (&Method::Post, "/api/tasks/conflict") => {
            let input: CreateDownloadTaskInput = read_json(request)?;
            with_task_service(context, |service| {
                json_response(StatusCode(200), &service.download_file_conflict(input)?)
            })
        }
        (&Method::Post, "/api/tasks/duplicate-check") => {
            let input: CreateDownloadTaskInput = read_json(request)?;
            with_task_service(context, |service| {
                json_response(StatusCode(200), &service.download_duplicate_check(input)?)
            })
        }
        (&Method::Post, "/api/tasks/refresh") => with_task_service(context, |service| {
            json_response(StatusCode(200), &service.refresh_all()?)
        }),
        (&Method::Post, "/api/tasks/pause") => {
            let input: TaskIdsInput = read_json(request)?;
            with_task_service(context, |service| {
                service.pause_tasks(&input.ids)?;
                empty_json_response()
            })
        }
        (&Method::Post, "/api/tasks/resume") => {
            let input: TaskIdsInput = read_json(request)?;
            with_task_service(context, |service| {
                service.resume_tasks(&input.ids)?;
                empty_json_response()
            })
        }
        (&Method::Post, "/api/tasks/delete") => {
            let input: DeleteTasksInput = read_json(request)?;
            with_task_service(context, |service| {
                service.delete_tasks(&input.ids, input.delete_completed_files)?;
                empty_json_response()
            })
        }
        (&Method::Post, "/api/tasks/clear-records") => {
            let input: ClearDownloadRecordsInput = read_json(request)?;
            with_task_service(context, |service| {
                json_response(
                    StatusCode(200),
                    &service.clear_download_records(input.older_than_days)?,
                )
            })
        }
        (&Method::Post, "/api/tasks/pause-all") => with_task_service(context, |service| {
            service.pause_all_unfinished()?;
            empty_json_response()
        }),
        (&Method::Post, "/api/tasks/resume-all") => with_task_service(context, |service| {
            service.resume_all_paused()?;
            empty_json_response()
        }),
        (&Method::Post, "/api/torrent-files") => {
            let input: TorrentFilesInput = read_json(request)?;
            let files = match input.source_type {
                SourceType::Torrent => crate::torrent_metadata::read_torrent_files(&input.source)?,
                SourceType::Magnet => {
                    let engine_settings_id = input
                        .engine_settings_id
                        .ok_or("engine settings id is required")?;
                    let save_path = input.save_path.ok_or("save path is required")?;
                    let connection = db::connect_path(context.database_path.clone())?;
                    let settings = crate::services::EngineSettingsService::new(&connection)
                        .get(&engine_settings_id)?;
                    if !settings.enabled {
                        return Err(format!("{} is disabled", settings.id).into());
                    }
                    crate::engine_adapters::resolve_magnet_files(
                        &settings,
                        &input.source,
                        &save_path,
                    )?
                    .unwrap_or_default()
                }
                _ => {
                    return Err(format!(
                        "{} does not have torrent files",
                        input.source_type.as_db()
                    )
                    .into())
                }
            };
            json_response(StatusCode(200), &files)
        }
        (&Method::Get, path)
            if path.starts_with("/api/tasks/") && path.ends_with("/torrent-files") =>
        {
            let id = path
                .trim_start_matches("/api/tasks/")
                .trim_end_matches("/torrent-files")
                .trim_end_matches('/');
            with_task_service(context, |service| {
                json_response(StatusCode(200), &service.torrent_files(id)?)
            })
        }
        (&Method::Post, path)
            if path.starts_with("/api/tasks/") && path.ends_with("/file-selection") =>
        {
            let id = path
                .trim_start_matches("/api/tasks/")
                .trim_end_matches("/file-selection")
                .trim_end_matches('/');
            let input: TaskFileSelectionInput = read_json(request)?;
            with_task_service(context, |service| {
                json_response(
                    StatusCode(200),
                    &service.update_file_selection(id, input.selected_file_indexes)?,
                )
            })
        }
        (&Method::Post, path) if path.starts_with("/api/tasks/") && path.ends_with("/open") => {
            let id = path
                .trim_start_matches("/api/tasks/")
                .trim_end_matches("/open")
                .trim_end_matches('/');
            with_task_service(context, |service| {
                service.open_downloaded_file(id)?;
                empty_json_response()
            })
        }
        (&Method::Post, path)
            if path.starts_with("/api/tasks/") && path.ends_with("/open-directory") =>
        {
            let id = path
                .trim_start_matches("/api/tasks/")
                .trim_end_matches("/open-directory")
                .trim_end_matches('/');
            with_task_service(context, |service| {
                service.open_download_directory(id)?;
                empty_json_response()
            })
        }
        (&Method::Get, "/api/app-settings") => {
            let connection = db::connect_path(context.database_path.clone())?;
            json_response(
                StatusCode(200),
                &crate::services::AppSettingsService::new(&connection).get()?,
            )
        }
        (&Method::Post, "/api/app-settings") => {
            let input: AppSettingsInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            let next = crate::services::AppSettingsService::new(&connection).save(input)?;
            drop(connection);
            if let Some(app_handle) = &context.app_handle {
                let state = app_handle.state::<crate::AppState>();
                let current = state
                    .app_settings()
                    .map_err(|error| -> Box<dyn Error> { error.into() })?;
                state
                    .set_app_settings(next.clone())
                    .map_err(|error| -> Box<dyn Error> { error.into() })?;
                state
                    .apply_web_settings_if_changed(app_handle.clone(), &current, &next)
                    .map_err(|error| -> Box<dyn Error> { error.into() })?;
                state
                    .refresh_sleep_prevention()
                    .map_err(|error| -> Box<dyn Error> { error.into() })?;
                state
                    .apply_auto_download_task_cleanup_for_settings(app_handle, &next)
                    .map_err(|error| -> Box<dyn Error> { error.into() })?;
            }
            json_response(StatusCode(200), &next)
        }
        (&Method::Get, "/api/engine-settings") => {
            let connection = db::connect_path(context.database_path.clone())?;
            json_response(
                StatusCode(200),
                &crate::services::EngineSettingsService::new(&connection).list_all()?,
            )
        }
        (&Method::Post, "/api/engine-settings") => {
            let input: crate::models::EngineSettingsInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            json_response(
                StatusCode(200),
                &crate::services::EngineSettingsService::new(&connection).save(input)?,
            )
        }
        (&Method::Post, path)
            if path.starts_with("/api/engine-settings/") && path.ends_with("/install-latest") =>
        {
            let settings_id = path
                .trim_start_matches("/api/engine-settings/")
                .trim_end_matches("/install-latest")
                .trim_end_matches('/');
            let connection = db::connect_path(context.database_path.clone())?;
            json_response(
                StatusCode(200),
                &crate::services::EngineSettingsService::new(&connection)
                    .install_latest(settings_id)?,
            )
        }
        (&Method::Delete, path) if path.starts_with("/api/engine-settings/") => {
            let settings_id = path.trim_start_matches("/api/engine-settings/");
            let connection = db::connect_path(context.database_path.clone())?;
            crate::services::EngineSettingsService::new(&connection).delete(settings_id)?;
            empty_json_response()
        }
        (&Method::Post, path)
            if path.starts_with("/api/engine-settings/") && path.ends_with("/trackers") =>
        {
            let settings_id = path
                .trim_start_matches("/api/engine-settings/")
                .trim_end_matches("/trackers")
                .trim_end_matches('/');
            let input: TrackerSubscriptionInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            json_response(
                StatusCode(200),
                &crate::services::EngineSettingsService::new(&connection)
                    .update_tracker_subscription(settings_id, &input.subscription_url)?,
            )
        }
        (&Method::Post, "/api/remote-directories") => {
            let input: RemoteDirectoriesInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            let settings = crate::services::EngineSettingsService::new(&connection)
                .get(&input.engine_settings_id)?;
            if !settings.enabled {
                return Err(format!("{} is disabled", settings.id).into());
            }
            json_response(
                StatusCode(200),
                &crate::engine_adapters::list_remote_directories(&settings, &input.path)?,
            )
        }
        (&Method::Post, "/api/magnet-name") => {
            let input: MagnetNameInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            let settings = crate::services::EngineSettingsService::new(&connection)
                .get(&input.engine_settings_id)?;
            if !settings.enabled {
                return Err(format!("{} is disabled", settings.id).into());
            }
            let metadata = crate::engine_adapters::resolve_magnet_metadata(
                &settings,
                &input.source,
                &input.save_path,
            )?;
            json_response(StatusCode(200), &metadata.name.unwrap_or_default())
        }
        (&Method::Post, "/api/test-engine-connection") => {
            let input: crate::models::EngineSettingsInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            crate::services::EngineSettingsService::new(&connection).test_connection(input)?;
            empty_json_response()
        }
        (&Method::Post, "/api/validate-engine-source-type") => {
            let input: ValidateEngineSourceTypeInput = read_json(request)?;
            let connection = db::connect_path(context.database_path.clone())?;
            crate::services::EngineSettingsService::new(&connection)
                .validate_source_type(input.engine, input.source_type)?;
            empty_json_response()
        }
        (&Method::Get, "/api/system-download-dir") => {
            let app_handle = context
                .app_handle
                .as_ref()
                .ok_or("system download directory requires app handle")?;
            json_response(
                StatusCode(200),
                &app_handle
                    .path()
                    .download_dir()?
                    .to_string_lossy()
                    .into_owned(),
            )
        }
        (&Method::Post, "/api/managed-engine-executable-path") => {
            let input: ManagedEngineExecutablePathInput = read_json(request)?;
            json_response(
                StatusCode(200),
                &engine_install::managed_executable_path(input.engine)
                    .map(|path| path.to_string_lossy().into_owned()),
            )
        }
        _ => json_response(
            StatusCode(404),
            &ErrorOutput {
                error: "not found".to_string(),
            },
        ),
    }
}

struct TaskEventStream {
    database_path: PathBuf,
    stop: Arc<AtomicBool>,
    buffer: Cursor<Vec<u8>>,
    sent_once: bool,
}

impl TaskEventStream {
    fn new(database_path: PathBuf, stop: Arc<AtomicBool>) -> Self {
        Self {
            database_path,
            stop,
            buffer: Cursor::new(Vec::new()),
            sent_once: false,
        }
    }

    fn next_event(&mut self) -> std::io::Result<Vec<u8>> {
        if self.sent_once {
            thread::sleep(Duration::from_secs(2));
        }
        self.sent_once = true;

        let payload = match self.load_tasks() {
            Ok(tasks) => serde_json::json!({ "tasks": tasks }),
            Err(error) => serde_json::json!({ "error": error.to_string() }),
        };
        let data = serde_json::to_string(&payload).map_err(std::io::Error::other)?;
        Ok(format!(
            "event: {}\ndata: {data}\n\n",
            task_events::DOWNLOAD_TASKS_UPDATED_EVENT
        )
        .into_bytes())
    }

    fn load_tasks(&self) -> Result<Vec<crate::models::DownloadTask>, Box<dyn Error>> {
        let connection = db::connect_path(self.database_path.clone())?;
        DownloadTaskService::new(&connection, self.database_path.clone()).refresh_all()
    }
}

impl Read for TaskEventStream {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        while self.buffer.position() as usize >= self.buffer.get_ref().len() {
            if self.stop.load(Ordering::Relaxed) {
                return Ok(0);
            }
            self.buffer = Cursor::new(self.next_event()?);
        }

        self.buffer.read(output)
    }
}

fn with_task_service<T>(
    context: &WebServerContext,
    run: impl FnOnce(&DownloadTaskService<'_>) -> Result<T, Box<dyn Error>>,
) -> Result<T, Box<dyn Error>> {
    let connection = db::connect_path(context.database_path.clone())?;
    let service = DownloadTaskService::new(&connection, context.database_path.clone());
    run(&service)
}

fn read_json<T: for<'de> Deserialize<'de>>(request: &mut Request) -> Result<T, Box<dyn Error>> {
    let mut body = String::new();
    request.as_reader().read_to_string(&mut body)?;
    Ok(serde_json::from_str(&body)?)
}

fn is_authorized(request: &Request, context: &WebServerContext) -> bool {
    let token = request.headers().iter().find_map(|header| {
        if header.field.equiv("authorization") {
            let value = header.value.as_str();
            return value.strip_prefix("Bearer ").map(str::to_string);
        }
        if header.field.equiv("x-unidl-token") {
            return Some(header.value.as_str().to_string());
        }
        None
    });

    let Some(token) = token else {
        return false;
    };

    context
        .sessions
        .lock()
        .map(|sessions| sessions.contains(&token))
        .unwrap_or(false)
}

fn serve_frontend_asset(
    context: &WebServerContext,
    path: &str,
) -> Result<ResponseBox, Box<dyn Error>> {
    let app_handle = context
        .app_handle
        .as_ref()
        .ok_or("web frontend requires app handle")?;
    let asset_path = if path == "/" || path.is_empty() {
        "index.html".to_string()
    } else {
        path.trim_start_matches('/').to_string()
    };
    let asset = app_handle
        .asset_resolver()
        .get(asset_path.clone())
        .ok_or_else(|| format!("frontend asset not found: {asset_path}"))?;
    Ok(with_cors(
        Response::from_data(asset.bytes)
            .with_status_code(StatusCode(200))
            .with_header(header("content-type", &asset.mime_type)),
    )
    .boxed())
}
fn event_stream_response(context: &WebServerContext) -> ResponseBox {
    let stream = TaskEventStream::new(context.database_path.clone(), Arc::clone(&context.stop));
    with_cors(
        Response::new(
            StatusCode(200),
            vec![
                header("content-type", "text/event-stream"),
                header("cache-control", "no-cache"),
            ],
            stream,
            None,
            None,
        )
        .with_chunked_threshold(1),
    )
    .boxed()
}

fn empty_json_response() -> Result<ResponseBox, Box<dyn Error>> {
    json_response(StatusCode(200), &serde_json::json!({}))
}

fn empty_response(status: StatusCode) -> ResponseBox {
    with_cors(Response::from_data(Vec::new()).with_status_code(status)).boxed()
}

fn json_response<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Result<ResponseBox, Box<dyn Error>> {
    let body = serde_json::to_vec(value)?;
    Ok(with_cors(
        Response::from_data(body)
            .with_status_code(status)
            .with_header(header("content-type", "application/json")),
    ))
    .map(Response::boxed)
}

fn with_cors<R: Read>(response: Response<R>) -> Response<R> {
    response
        .with_header(header("access-control-allow-origin", "*"))
        .with_header(header(
            "access-control-allow-methods",
            "GET, POST, DELETE, OPTIONS",
        ))
        .with_header(header(
            "access-control-allow-headers",
            "authorization, content-type, x-unidl-token",
        ))
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("invalid HTTP header")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        net::TcpListener,
        path::PathBuf,
        sync::{Arc, Mutex},
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use reqwest::{blocking::Client, StatusCode};
    use serde_json::Value;
    use uuid::Uuid;

    use super::*;
    use crate::db;

    #[test]
    fn local_api_requires_login_and_lists_tasks() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        drop(connection);

        let bind_address = free_bind_address();
        let base_url = format!("http://{bind_address}");
        let pending_open_sources = Arc::new(Mutex::new(Vec::new()));
        let handle = start_on(
            &bind_address,
            None,
            database_path.clone(),
            Arc::clone(&pending_open_sources),
            &app_settings(true, "secret"),
        )
        .expect("web server should start");
        let client = Client::new();

        let health = client
            .get(format!("{base_url}/api/health"))
            .send()
            .expect("health request should succeed");
        assert_eq!(health.status(), StatusCode::OK);

        let unauthorized = client
            .get(format!("{base_url}/api/tasks"))
            .send()
            .expect("tasks request should complete");
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let rejected = client
            .post(format!("{base_url}/api/login"))
            .json(&serde_json::json!({ "password": "wrong" }))
            .send()
            .expect("login request should complete");
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

        let login: Value = client
            .post(format!("{base_url}/api/login"))
            .json(&serde_json::json!({ "password": "secret" }))
            .send()
            .expect("login request should complete")
            .json()
            .expect("login response should be json");
        let token = login["token"].as_str().expect("login should return token");

        let tasks: Value = client
            .get(format!("{base_url}/api/tasks"))
            .bearer_auth(token)
            .send()
            .expect("authorized tasks request should complete")
            .json()
            .expect("tasks response should be json");
        assert_eq!(tasks.as_array().expect("tasks should be an array").len(), 0);

        handle.stop();
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn extension_task_endpoint_queues_source_for_new_task_dialog_without_login() {
        let database_path = temp_database_path();
        let connection = db::connect_path(database_path.clone()).expect("database should migrate");
        drop(connection);

        let bind_address = free_bind_address();
        let base_url = format!("http://{bind_address}");
        let pending_open_sources = Arc::new(Mutex::new(Vec::new()));
        let handle = start_on(
            &bind_address,
            None,
            database_path.clone(),
            Arc::clone(&pending_open_sources),
            &app_settings(false, "secret"),
        )
        .expect("web server should start");
        let client = Client::new();

        let health = client
            .get(format!("{base_url}/api/health"))
            .send()
            .expect("health request should succeed");
        assert_eq!(health.status(), StatusCode::OK);

        let disabled_login = client
            .post(format!("{base_url}/api/login"))
            .json(&serde_json::json!({ "password": "secret" }))
            .send()
            .expect("disabled login request should complete");
        assert_eq!(disabled_login.status(), StatusCode::FORBIDDEN);

        let task: Value = client
            .post(format!("{base_url}/api/extension/tasks"))
            .json(&serde_json::json!({
                "sourceType": "magnet",
                "source": "magnet:?xt=urn:btih:ABCDEF123456&dn=unidl",
                "fileName": "unidl",
                "httpReferrer": "https://example.test/page"
            }))
            .send()
            .expect("extension task request should complete")
            .json()
            .expect("extension task response should be json");

        assert_eq!(task["fileName"], "unidl");
        assert_eq!(
            pending_open_sources
                .lock()
                .expect("pending sources should lock")
                .as_slice(),
            &[system_open::OpenTaskRequest {
                source: "magnet:?xt=urn:btih:ABCDEF123456&dn=unidl".to_string(),
                file_name: Some("unidl".to_string()),
                browser_cookies: None,
                http_referrer: Some("https://example.test/page".to_string()),
            }]
        );

        handle.stop();
        let _ = fs::remove_file(database_path);
    }

    #[test]
    fn extension_videos_only_count_when_ytdlp_extracts_entries() {
        let script_path = create_fake_ytdlp_script(
            r#"{"title":"Playlist","entries":[{"id":"1","title":"Video 1","url":"https://example.com/watch?v=1"},{"id":"2","title":"Video 2","url":"https://example.com/watch?v=2"}]}"#,
        );

        let videos = extract_extension_videos(
            script_path.to_str().expect("script path should be utf-8"),
            "https://example.com/watch",
            Some("Example page"),
            None,
            None,
        )
        .expect("video extraction should succeed");

        assert_eq!(videos.len(), 2);
        assert_eq!(videos[0].title, "Video 1");
        assert_eq!(videos[0].source, "https://example.com/watch?v=1");
        assert_eq!(videos[1].title, "Video 2");
        assert_eq!(videos[1].source, "https://example.com/watch?v=2");

        let _ = fs::remove_file(script_path);
    }

    fn create_fake_ytdlp_script(json_output: &str) -> PathBuf {
        let script_path = if cfg!(windows) {
            std::env::temp_dir().join(format!("unidl-test-ytdlp-{}.cmd", Uuid::new_v4()))
        } else {
            std::env::temp_dir().join(format!("unidl-test-ytdlp-{}.sh", Uuid::new_v4()))
        };

        let script = if cfg!(windows) {
            format!("@echo off\r\necho {}\r\n", json_output)
        } else {
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", json_output)
        };

        fs::write(&script_path, script).expect("script should write");
        #[cfg(unix)]
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755))
            .expect("script should be executable");

        script_path
    }
    fn free_bind_address() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test port should bind");
        let address = listener
            .local_addr()
            .expect("test port should have address");
        drop(listener);
        address.to_string()
    }

    fn temp_database_path() -> PathBuf {
        std::env::temp_dir().join(format!("unidl-test-{}.sqlite3", Uuid::new_v4()))
    }

    fn app_settings(web_access_enabled: bool, password: &str) -> AppSettings {
        AppSettings {
            web_access_enabled,
            web_access_password: password.to_string(),
            web_access_url: "http://127.0.0.1:18080".to_string(),
            private_download_domains: Vec::new(),
            app_proxy_url: String::new(),
            auto_start_enabled: false,
            auto_start_minimized_to_tray: false,
            close_to_tray_enabled: false,
            download_completion_notification_enabled: false,
            prevent_sleep_when_downloading_enabled: false,
            prevent_sleep_when_web_access_enabled: false,
            local_download_concurrency: 5,
            auto_clean_download_tasks_enabled: false,
            auto_clean_download_tasks_days: 365,
        }
    }
}
