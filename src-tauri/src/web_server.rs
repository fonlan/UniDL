use std::{
    collections::HashSet,
    error::Error,
    io::{Cursor, Read},
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tiny_http::{Header, Method, Request, Response, ResponseBox, Server, StatusCode};
use uuid::Uuid;

use crate::{
    db,
    models::{AppSettings, CreateDownloadTaskInput, EngineKind, SourceType},
    repositories::EngineSettingsRepository,
    services::DownloadTaskService,
    system_open,
    task_events,
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
struct ExtensionVideosInput {
    source: String,
    title: Option<String>,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionYtDlpTaskInput {
    source: String,
    file_name: String,
    cookies: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExtensionTaskInput {
    source_type: SourceType,
    source: String,
    file_name: String,
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
                Err(error) => eprintln!("web server request error: {error}"),
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
        Err(error) => json_response(
            StatusCode(500),
            &ErrorOutput {
                error: error.to_string(),
            },
        )
        .expect("failed to build error response"),
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

fn handle_extension_videos(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionVideosInput = read_json(request)?;
    let source = input.source.trim();
    let Some(source_type) = extension_source_type(source) else {
        return json_response(StatusCode(200), &ExtensionVideosOutput { videos: Vec::new() });
    };

    if find_enabled_ytdlp_settings(context, source_type)?.is_none() {
        return json_response(StatusCode(200), &ExtensionVideosOutput { videos: Vec::new() });
    }

    json_response(
        StatusCode(200),
        &ExtensionVideosOutput {
            videos: vec![ExtensionVideoEntry {
                id: "current-page".to_string(),
                title: input
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| extension_video_title(source)),
                source: source.to_string(),
            }],
        },
    )
}

fn handle_extension_ytdlp_task(
    request: &mut Request,
    context: &WebServerContext,
) -> Result<ResponseBox, Box<dyn Error>> {
    let input: ExtensionYtDlpTaskInput = read_json(request)?;
    let source = input.source.trim().to_string();
    let file_name = input.file_name.trim().to_string();
    let source_type = extension_source_type(&source).ok_or("yt-dlp source must be http or https")?;
    find_enabled_ytdlp_settings(context, source_type)?
        .ok_or("no enabled yt-dlp settings supports this page")?;

    open_task_dialog(
        context,
        system_open::OpenTaskRequest {
            source,
            file_name: Some(file_name.clone()),
            browser_cookies: input.cookies.filter(|value| !value.trim().is_empty()),
        },
    )?;
    json_response(
        StatusCode(200),
        &ExtensionTaskOutput {
            file_name,
        },
    )
}

fn find_enabled_ytdlp_settings(
    context: &WebServerContext,
    source_type: SourceType,
) -> Result<Option<crate::models::EngineSettings>, Box<dyn Error>> {
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

fn extension_source_type(source: &str) -> Option<SourceType> {
    if source.starts_with("http://") || source.starts_with("https://") {
        Some(SourceType::Http)
    } else {
        None
    }
}

fn extension_video_title(source: &str) -> String {
    source
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split(|value| value == char::from(47) || value == char::from(63) || value == char::from(35))
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
            let input: TaskIdsInput = read_json(request)?;
            with_task_service(context, |service| {
                service.delete_tasks(&input.ids, false)?;
                empty_json_response()
            })
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
        Ok(format!("event: {}\ndata: {data}\n\n", task_events::DOWNLOAD_TASKS_UPDATED_EVENT).into_bytes())
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
        .with_header(header("access-control-allow-methods", "GET, POST, OPTIONS"))
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
                "fileName": "unidl"
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
            }]
        );

        handle.stop();
        let _ = fs::remove_file(database_path);
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
        }
    }
}
