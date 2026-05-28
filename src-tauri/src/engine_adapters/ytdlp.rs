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
    time::Duration,
};

use serde_json::Value;

use crate::{
    logger,
    models::{DownloadStatus, DownloadTask, EngineSettings},
    repositories::DownloadTaskRepository,
};

use super::{
    append_args, engine_proxy_url, engine_speed_limit_bytes_per_sec, engine_user_agent,
    log_command, required_engine_task_id, EngineTaskState,
};

const YTDLP_FAST_DEFAULT_ARGS: &str = "--no-playlist --js-runtimes node --concurrent-fragments 8";
const YTDLP_PROGRESS_PREFIX: &str = "[UniDL:progress] ";
const YTDLP_PROGRESS_TEMPLATE: &str = r#"download:[UniDL:progress] {"status":%(progress.status|)j,"downloadedBytes":%(progress.downloaded_bytes|0)j,"totalBytes":%(progress.total_bytes|0)j,"totalBytesEstimate":%(progress.total_bytes_estimate|0)j,"speedBytesPerSec":%(progress.speed|0)j,"percent":%(progress._percent_str|)j}"#;

pub(super) fn add_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
    force_continue: bool,
) -> Result<EngineTaskState, Box<dyn Error>> {
    add_ytdlp_task(settings, task, database_path, force_continue)
}

pub(super) fn pause_task(task: &DownloadTask) -> Result<EngineTaskState, Box<dyn Error>> {
    terminate_process(ytdlp_pid(task)?)?;
    Ok(EngineTaskState {
        status: DownloadStatus::Paused,
        progress: task.progress,
        speed_bytes_per_sec: 0,
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
        engine_task_id: None,
        file_name: None,
        error_message: None,
    })
}

pub(super) fn resume_task(
    settings: &EngineSettings,
    task: &DownloadTask,
    database_path: PathBuf,
) -> Result<EngineTaskState, Box<dyn Error>> {
    add_ytdlp_task(settings, task, database_path, true)
}

pub(super) fn delete_task(task: &DownloadTask) -> Result<(), Box<dyn Error>> {
    if !matches!(
        task.status,
        DownloadStatus::Completed | DownloadStatus::Failed | DownloadStatus::Paused
    ) {
        let pid = ytdlp_pid(task)?;
        terminate_process(pid)?;
    }
    Ok(())
}

pub(super) fn refresh_task(task: &DownloadTask) -> Result<EngineTaskState, Box<dyn Error>> {
    refresh_ytdlp_task(task)
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
    require_ytdlp_ffmpeg(settings, task)?;
    let mut command = Command::new(executable);
    apply_ytdlp_utf8_env(&mut command);
    let cookie_path = write_ytdlp_cookie_file(task)?;
    append_args(&mut command, YTDLP_FAST_DEFAULT_ARGS);
    append_args(&mut command, &settings.default_args);
    append_args(&mut command, &task.engine_args);
    append_ytdlp_transfer_options(
        &mut command,
        engine_user_agent(settings),
        engine_speed_limit_bytes_per_sec(settings),
    );
    append_ytdlp_progress_args(&mut command);
    if force_continue {
        command.arg("--continue");
    } else {
        command.arg("--force-overwrites");
    }
    if let Some(proxy_url) = engine_proxy_url(settings) {
        command.arg("--proxy").arg(proxy_url);
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
    let progress_pid = pid.clone();
    let progress_database_path = database_path.clone();
    let completion_pid = pid.clone();
    thread::spawn(move || {
        let output_lines = Arc::new(Mutex::new(VecDeque::new()));
        let stdout_reader = stdout.map(|output| {
            spawn_ytdlp_progress_reader(
                output,
                progress_database_path.clone(),
                progress_task_id.clone(),
                progress_pid.clone(),
                Arc::clone(&output_lines),
            )
        });
        let stderr_reader = stderr.map(|output| {
            spawn_ytdlp_progress_reader(
                output,
                progress_database_path,
                progress_task_id,
                progress_pid,
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
        let _ = update_ytdlp_completion(
            &database_path,
            &task_id,
            &completion_pid,
            status,
            error_message.as_deref(),
        );
    });

    Ok(EngineTaskState::running(pid))
}

fn require_ytdlp_ffmpeg(
    settings: &EngineSettings,
    task: &DownloadTask,
) -> Result<(), Box<dyn Error>> {
    if ytdlp_ffmpeg_available(settings, task) {
        return Ok(());
    }

    Err("yt-dlp requires ffmpeg, but ffmpeg was not found. Install ffmpeg, add it to PATH, place it next to yt-dlp, or set --ffmpeg-location.".into())
}

fn ytdlp_ffmpeg_available(settings: &EngineSettings, task: &DownloadTask) -> bool {
    if let Some(location) = ytdlp_ffmpeg_location(&task.engine_args)
        .or_else(|| ytdlp_ffmpeg_location(&settings.default_args))
    {
        return ytdlp_ffmpeg_location_exists(&location);
    }

    ytdlp_ffmpeg_next_to_executable(settings) || ytdlp_ffmpeg_on_path()
}

fn ytdlp_ffmpeg_location(args: &str) -> Option<String> {
    let mut parts = args.split_whitespace();
    while let Some(arg) = parts.next() {
        if arg == "--ffmpeg-location" {
            return parts.next().map(trim_ytdlp_arg).and_then(non_empty_string);
        }
        if let Some(location) = arg.strip_prefix("--ffmpeg-location=") {
            return non_empty_string(trim_ytdlp_arg(location));
        }
    }
    None
}

fn trim_ytdlp_arg(value: &str) -> &str {
    value.trim().trim_matches(['\"', '\''])
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn ytdlp_ffmpeg_location_exists(location: &str) -> bool {
    let path = Path::new(location);
    if path.is_dir() {
        return path.join(ffmpeg_binary_name()).is_file();
    }
    path.is_file()
}

fn ytdlp_ffmpeg_next_to_executable(settings: &EngineSettings) -> bool {
    let Some(executable) = settings
        .executable_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    Path::new(executable)
        .parent()
        .is_some_and(|directory| directory.join(ffmpeg_binary_name()).is_file())
}

fn ytdlp_ffmpeg_on_path() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn ffmpeg_binary_name() -> &'static str {
    if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

fn spawn_ytdlp_progress_reader(
    output: impl Read + Send + 'static,
    database_path: PathBuf,
    task_id: String,
    pid: String,
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
            let line = decode_ytdlp_stdout_line(&bytes);
            if let Some(progress) = parse_ytdlp_progress(&line) {
                let _ = update_ytdlp_progress(
                    &database_path,
                    &task_id,
                    &pid,
                    progress.percent,
                    progress.speed_bytes_per_sec,
                    progress.downloaded_bytes,
                    progress.total_bytes,
                );
            }
            if let Some(file_name) = parse_ytdlp_destination_name(&line) {
                if !is_ytdlp_format_temp_file_name(&file_name) {
                    let _ = update_ytdlp_file_name(&database_path, &task_id, &pid, &file_name);
                }
            }
            remember_ytdlp_output_line(&output_lines, line);
            bytes.clear();
        }
    })
}

/// Decode one stdout/stderr line from yt-dlp into a String, tolerating the
/// common Windows mojibake case where the PyInstaller-frozen yt-dlp.exe ignores
/// `PYTHONIOENCODING` and writes the active ANSI/OEM code page (e.g. CP936/GBK
/// on Chinese Windows) to its pipe. Strategy:
///   1. Try UTF-8 strict 鈥?works for any modern yt-dlp on any platform that
///      honours `PYTHONIOENCODING=utf-8`.
///   2. On Windows, fall back to the system ANSI code page (resolved by
///      `windows_ansi_encoding`), which covers all CJK/EE/etc. mojibake cases.
///   3. As a last resort, lossy UTF-8 (preserves ASCII, replaces the rest with
///      U+FFFD).
pub(crate) fn decode_ytdlp_stdout_line(bytes: &[u8]) -> String {
    if let Ok(value) = std::str::from_utf8(bytes) {
        return value.to_string();
    }
    #[cfg(windows)]
    {
        if let Some(encoding) = windows_ansi_encoding() {
            let (decoded, _, had_errors) = encoding.decode(bytes);
            if !had_errors {
                return decoded.into_owned();
            }
        }
    }
    String::from_utf8_lossy(bytes).into_owned()
}

#[cfg(windows)]
fn windows_ansi_encoding() -> Option<&'static encoding_rs::Encoding> {
    static CACHED: std::sync::OnceLock<Option<&'static encoding_rs::Encoding>> =
        std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        let acp = read_active_code_page().unwrap_or(936);
        encoding_for_windows_code_page(acp).or(Some(encoding_rs::GBK))
    })
}

#[cfg(windows)]
fn read_active_code_page() -> Option<u32> {
    let key = windows_registry::LOCAL_MACHINE
        .open(r"SYSTEM\CurrentControlSet\Control\Nls\CodePage")
        .ok()?;
    let acp: String = key.get_string("ACP").ok()?;
    acp.trim().parse::<u32>().ok()
}

#[cfg(windows)]
fn encoding_for_windows_code_page(code_page: u32) -> Option<&'static encoding_rs::Encoding> {
    // Map the most common Windows ANSI/OEM code pages to encoding_rs labels.
    // 936/950/932/949 are the four CJK ANSI pages that produce the worst
    // mojibake when decoded as UTF-8 鈥?the rest are EU/Latin variants that
    // happen to be ASCII-compatible for typical filenames.
    let label: &[u8] = match code_page {
        936 => b"GBK",
        950 => b"Big5",
        932 => b"Shift_JIS",
        949 => b"EUC-KR",
        874 => b"windows-874",
        1250 => b"windows-1250",
        1251 => b"windows-1251",
        1252 => b"windows-1252",
        1253 => b"windows-1253",
        1254 => b"windows-1254",
        1255 => b"windows-1255",
        1256 => b"windows-1256",
        1257 => b"windows-1257",
        1258 => b"windows-1258",
        _ => return None,
    };
    encoding_rs::Encoding::for_label(label)
}

/// Parse the file name out of yt-dlp's progress lines that announce a chosen
/// output path, so we can keep the task's display name in sync with the actual
/// file written to disk (the only place that knows the final extension).
///
/// Recognized line shapes (yt-dlp 2024+):
/// - `[download] Destination: <path>` 鈥?for any single stream, including the
///   final file in the non-merging case.
/// - `[Merger] Merging formats into "<path>"` 鈥?the post-merge final file.
/// - `[ExtractAudio] Destination: <path>` 鈥?audio-only / extracted-audio path.
pub(crate) fn parse_ytdlp_destination_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let path = if let Some(rest) = trimmed.strip_prefix("[download] Destination: ") {
        rest.trim().to_string()
    } else if let Some(rest) = trimmed.strip_prefix("[ExtractAudio] Destination: ") {
        rest.trim().to_string()
    } else if let Some(rest) = trimmed.strip_prefix("[Merger] Merging formats into ") {
        let rest = rest.trim();
        rest.strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .unwrap_or(rest)
            .to_string()
    } else {
        return None;
    };
    if path.is_empty() {
        return None;
    }
    Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

/// yt-dlp writes per-format streams as `<base>.f<digits>.<ext>` while it is
/// still in the middle of downloading audio + video separately. These names are
/// transient 鈥?the merger step will rename to `<base>.<ext>` 鈥?so we skip them
/// when syncing the task's display name.
pub(crate) fn is_ytdlp_format_temp_file_name(name: &str) -> bool {
    let mut parts = name.rsplitn(3, '.');
    let _ext = match parts.next() {
        Some(value) if !value.is_empty() => value,
        _ => return false,
    };
    let Some(format_part) = parts.next() else {
        return false;
    };
    if parts.next().is_none() {
        return false;
    }
    let Some(digits) = format_part.strip_prefix('f') else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

fn update_ytdlp_file_name(
    database_path: &Path,
    task_id: &str,
    pid: &str,
    file_name: &str,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    if !ytdlp_spawn_owns_task(&repository, task_id, pid)? {
        // Same rationale as update_ytdlp_progress: a stale spawn thread must
        // not rename the file of a paused / deleted / re-spawned task.
        return Ok(());
    }
    repository.update_file_name(task_id, file_name)?;
    Ok(())
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

pub(super) fn ytdlp_output_template(file_name: &str) -> String {
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

/// Force yt-dlp.exe (PyInstaller-frozen Python) to emit UTF-8 on stdout / stderr.
/// Without this, on a Chinese / non-en Windows host yt-dlp falls back to the
/// system OEM code page (e.g. CP936/GBK) for piped output, so any Chinese path
/// or title we parse out of progress lines becomes mojibake when we decode it
/// via `from_utf8_lossy`. Setting both vars covers all yt-dlp builds we ship.
pub(crate) fn apply_ytdlp_utf8_env(command: &mut Command) {
    command.env("PYTHONIOENCODING", "utf-8");
    command.env("PYTHONUTF8", "1");
}

fn append_ytdlp_progress_args(command: &mut Command) {
    command
        .arg("--progress")
        .arg("--newline")
        .arg("--progress-template")
        .arg(YTDLP_PROGRESS_TEMPLATE);
}

pub(super) fn append_ytdlp_transfer_options(
    command: &mut Command,
    user_agent: Option<&str>,
    speed_limit_bytes_per_sec: Option<i64>,
) {
    if let Some(user_agent) = user_agent.map(str::trim).filter(|value| !value.is_empty()) {
        command.arg("--user-agent").arg(user_agent);
    }
    if let Some(speed_limit_bytes_per_sec) = speed_limit_bytes_per_sec.filter(|value| *value > 0) {
        command
            .arg("--limit-rate")
            .arg(speed_limit_bytes_per_sec.to_string());
    }
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
    // For yt-dlp the background thread spawned by `add_ytdlp_task` is the
    // ONLY authoritative writer of status transitions: it owns `child.wait()`
    // and `update_ytdlp_completion`. Refresh must therefore be a pure
    // read-back / no-op echo for status. Any attempt here to "infer" a
    // status from a tasklist probe creates the symmetric pair of bugs we
    // already had to fix:
    //   * the previous bug: `pid_alive == false` flipped Running -> Failed
    //     (fixed in commit 12b2550 by guarding the downgrade);
    //   * the current bug: `pid_alive == true` flipped Failed -> Running
    //     (observed when pause's `taskkill` failed and the user's task was
    //     marked Failed by `services::pause_tasks::mark_failed`, then
    //     refresh saw the process still alive and revived it back to
    //     Running, where it eventually finished as Completed -- exactly
    //     the "Failed -> Running -> still downloading" flicker reported).
    //
    // The right answer in both directions is: refresh does not invent
    // status for yt-dlp. Speed is zeroed when the row is not Running so
    // the UI doesn't keep showing a stale rate after a terminal state.
    let speed_bytes_per_sec = if task.status == DownloadStatus::Running {
        task.speed_bytes_per_sec
    } else {
        0
    };
    Ok(EngineTaskState {
        status: task.status,
        progress: task.progress,
        speed_bytes_per_sec,
        downloaded_bytes: task.downloaded_bytes,
        total_bytes: task.total_bytes,
        engine_task_id: task.engine_task_id.clone(),
        file_name: None,
        error_message: task.error_message.clone(),
    })
}

/// Probe `tasklist` to decide whether a yt-dlp PID is still alive.
/// Used by `terminate_process` to short-circuit retries when the OS already
/// reports the process as gone (e.g. taskkill returned non-zero with
/// "process not found" because the child exited just before our retry).
///
/// Failure modes are deliberately conservative: any error path returns
/// `true` (= "assume alive") so the caller errs on the side of one more
/// retry rather than silently treating a flaky tasklist as success.
fn ytdlp_pid_appears_alive(pid: &str) -> bool {
    let Ok(output) = Command::new("tasklist")
        .args(["/NH", "/FO", "CSV", "/FI", &format!("PID eq {pid}")])
        .output()
    else {
        return true;
    };
    if !output.status.success() {
        return true;
    }
    String::from_utf8_lossy(&output.stdout).contains(pid)
}

#[derive(Debug, PartialEq)]
pub(super) struct YtDlpProgress {
    pub(super) percent: f64,
    pub(super) speed_bytes_per_sec: i64,
    pub(super) downloaded_bytes: i64,
    pub(super) total_bytes: i64,
}

pub(super) fn parse_ytdlp_progress(line: &str) -> Option<YtDlpProgress> {
    if let Some(progress) = parse_ytdlp_progress_template(line) {
        return Some(progress);
    }

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
        .iter()
        .enumerate()
        .find_map(|(index, part)| {
            if *part != "of" {
                return None;
            }
            let value = parts.get(index + 1)?;
            let value = if *value == "~" {
                parts.get(index + 2)?
            } else {
                value
            };
            parse_ytdlp_byte_value(value)
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

fn parse_ytdlp_progress_template(line: &str) -> Option<YtDlpProgress> {
    let payload = line.trim().strip_prefix(YTDLP_PROGRESS_PREFIX)?;
    let value: Value = serde_json::from_str(payload).ok()?;
    let downloaded_bytes = ytdlp_json_i64(value.get("downloadedBytes"))
        .unwrap_or(0)
        .max(0);
    let total_bytes = ytdlp_json_i64(value.get("totalBytes"))
        .filter(|total| *total > 0)
        .or_else(|| ytdlp_json_i64(value.get("totalBytesEstimate")).filter(|total| *total > 0))
        .unwrap_or(0);
    let speed_bytes_per_sec = ytdlp_json_i64(value.get("speedBytesPerSec"))
        .unwrap_or(0)
        .max(0);
    let percent = ytdlp_json_f64(value.get("percent"))
        .or_else(|| {
            value
                .get("percent")
                .and_then(Value::as_str)
                .and_then(parse_ytdlp_percent_value)
        })
        .or_else(|| {
            if total_bytes > 0 {
                Some((downloaded_bytes as f64 / total_bytes as f64) * 100.0)
            } else {
                None
            }
        })
        .unwrap_or(0.0);

    Some(YtDlpProgress {
        percent,
        speed_bytes_per_sec,
        downloaded_bytes,
        total_bytes,
    })
}

fn ytdlp_json_i64(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| number.as_f64().map(|value| value.round() as i64)),
        Value::String(value) => value
            .trim()
            .parse::<f64>()
            .ok()
            .map(|value| value.round() as i64),
        _ => None,
    }
}

fn ytdlp_json_f64(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(value) => {
            parse_ytdlp_percent_value(value).or_else(|| value.trim().parse::<f64>().ok())
        }
        _ => None,
    }
}

fn parse_ytdlp_percent_value(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix('%')
        .unwrap_or_else(|| value.trim())
        .trim()
        .parse::<f64>()
        .ok()
}

pub(super) fn parse_ytdlp_speed(value: &str) -> Option<i64> {
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

pub(super) fn update_ytdlp_progress(
    database_path: &Path,
    task_id: &str,
    pid: &str,
    progress: f64,
    speed_bytes_per_sec: i64,
    downloaded_bytes: i64,
    total_bytes: i64,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    if !ytdlp_spawn_owns_task(&repository, task_id, pid)? {
        // The spawn thread that owns `pid` no longer reflects the active session
        // (user paused, deleted, or already resumed into a new spawn). Letting
        // a flushed-from-pipe progress line clobber Paused/Failed/Completed
        // back to Running is exactly the bug we're avoiding here.
        return Ok(());
    }
    repository.update_engine_state(
        task_id,
        DownloadStatus::Running,
        progress,
        speed_bytes_per_sec,
        downloaded_bytes,
        total_bytes,
        Some(pid),
        None,
    )?;
    Ok(())
}

pub(super) fn update_ytdlp_completion(
    database_path: &Path,
    task_id: &str,
    pid: &str,
    status: DownloadStatus,
    error_message: Option<&str>,
) -> Result<(), Box<dyn Error>> {
    let connection = rusqlite::Connection::open(database_path)?;
    let repository = DownloadTaskRepository::new(&connection);
    let task = repository.get_by_id(task_id)?;
    if !ytdlp_spawn_owns_task_with(&task, pid) {
        // User-initiated pause / delete / resume into a new spawn must not be
        // overwritten by the previous spawn's terminal status (typically a
        // bogus Failed because pause used taskkill /F to stop the process).
        return Ok(());
    }
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
        Some(pid),
        error_message,
    )?;
    Ok(())
}

/// True only if the live DB row says: (1) status is Running, AND (2) the
/// engine_task_id (= yt-dlp PID we wrote at spawn time) still matches `pid`.
/// Any other state 鈥?Paused (user pause), Completed/Failed (terminal),
/// Deleted (user delete), or a different PID (a newer spawn already took
/// over) 鈥?means the spawn thread that's calling us is stale and must NOT
/// touch the DB.
fn ytdlp_spawn_owns_task(
    repository: &DownloadTaskRepository<'_>,
    task_id: &str,
    pid: &str,
) -> Result<bool, Box<dyn Error>> {
    let task = repository.get_by_id(task_id)?;
    Ok(ytdlp_spawn_owns_task_with(&task, pid))
}

fn ytdlp_spawn_owns_task_with(task: &DownloadTask, pid: &str) -> bool {
    if task.status != DownloadStatus::Running {
        return false;
    }
    matches!(task.engine_task_id.as_deref(), Some(current) if current == pid)
}

fn ytdlp_pid(task: &DownloadTask) -> Result<NonZeroU32, Box<dyn Error>> {
    let pid = required_engine_task_id(task)?
        .parse::<NonZeroU32>()
        .map_err(|_| format!("task {} has invalid yt-dlp pid", task.id))?;
    Ok(pid)
}

/// Stop a running yt-dlp process tree on Windows.
///
/// Why this is a retry loop and not a single `taskkill`:
/// in the wild, `taskkill /T /F` can return non-zero on a yt-dlp child
/// while the user is pausing -- typically a transient "Access is denied"
/// during the ffmpeg merge stage, or a TOCTOU race where one of the
/// child PIDs in the tree exits between the kernel's enumeration and the
/// kill attempt. Without retries the user sees their pause request
/// surface as "failed to stop yt-dlp process N", `services::pause_tasks`
/// then marks the task Failed, the process happily keeps downloading,
/// and refresh later flips the row back to Running -- exactly the
/// "Failed -> Running -> still downloading" flicker reported.
///
/// Strategy:
/// 1. Run `taskkill /T /F` and capture stderr so any real error makes
///    it into the surfaced message.
/// 2. If `taskkill` succeeded, we're done.
/// 3. If `taskkill` failed but the process is no longer in `tasklist`,
///    the user's intent is satisfied -- treat as success. Covers the
///    common "process not found" exit that taskkill emits as non-zero.
/// 4. Otherwise sleep briefly and retry, up to a small bound.
#[cfg(windows)]
fn terminate_process(pid: NonZeroU32) -> Result<(), Box<dyn Error>> {
    const ATTEMPTS: usize = 3;
    const RETRY_BACKOFF: Duration = Duration::from_millis(250);

    let pid_str = pid.to_string();
    let mut last_stderr = String::new();
    for attempt in 0..ATTEMPTS {
        if attempt > 0 {
            thread::sleep(RETRY_BACKOFF);
        }
        let output = Command::new("taskkill")
            .args(["/PID", &pid_str, "/T", "/F"])
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        if !ytdlp_pid_appears_alive(&pid_str) {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !stderr.is_empty() {
            last_stderr = stderr;
        }
    }
    let detail = if last_stderr.is_empty() {
        "<no taskkill stderr>".to_string()
    } else {
        last_stderr
    };
    Err(format!(
        "failed to stop yt-dlp process {} after {} attempts: {}",
        pid, ATTEMPTS, detail
    )
    .into())
}

#[cfg(not(windows))]
fn terminate_process(_pid: NonZeroU32) -> Result<(), Box<dyn Error>> {
    Err("yt-dlp pause is only supported on Windows".into())
}
