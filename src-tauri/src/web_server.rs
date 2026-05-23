use std::{
    collections::HashSet,
    error::Error,
    io::Read,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

use crate::{db, models::CreateDownloadTaskInput, services::DownloadTaskService};

pub const WEB_ACCESS_URL: &str = "http://127.0.0.1:18080";

const WEB_BIND_ADDRESS: &str = "127.0.0.1:18080";

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
    database_path: PathBuf,
    password: String,
    sessions: Arc<Mutex<HashSet<String>>>,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorOutput {
    error: String,
}

pub fn start(
    database_path: PathBuf,
    password: String,
) -> Result<WebServerHandle, Box<dyn Error + Send + Sync>> {
    let server = Server::http(WEB_BIND_ADDRESS)?;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let context = WebServerContext {
        database_path,
        password,
        sessions: Arc::new(Mutex::new(HashSet::new())),
    };

    let thread = thread::spawn(move || {
        while !thread_stop.load(Ordering::Relaxed) {
            match server.recv_timeout(Duration::from_millis(250)) {
                Ok(Some(request)) => handle_request(request, &context),
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
) -> Result<Response<std::io::Cursor<Vec<u8>>>, Box<dyn Error>> {
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

fn handle_authorized_request(
    request: &mut Request,
    context: &WebServerContext,
    method: &Method,
    path: &str,
) -> Result<Response<std::io::Cursor<Vec<u8>>>, Box<dyn Error>> {
    match (method, path) {
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
                service.delete_tasks(&input.ids)?;
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

fn empty_json_response() -> Result<Response<std::io::Cursor<Vec<u8>>>, Box<dyn Error>> {
    json_response(StatusCode(200), &serde_json::json!({}))
}

fn empty_response(status: StatusCode) -> Response<std::io::Cursor<Vec<u8>>> {
    with_cors(Response::from_data(Vec::new()).with_status_code(status))
}

fn json_response<T: Serialize>(
    status: StatusCode,
    value: &T,
) -> Result<Response<std::io::Cursor<Vec<u8>>>, Box<dyn Error>> {
    let body = serde_json::to_vec(value)?;
    Ok(with_cors(
        Response::from_data(body)
            .with_status_code(status)
            .with_header(header("content-type", "application/json")),
    ))
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
