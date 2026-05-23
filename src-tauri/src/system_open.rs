use std::error::Error;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

pub const SYSTEM_OPEN_EVENT: &str = "system-open-request";

#[derive(Clone, Serialize)]
pub struct OpenRequestPayload {
    sources: Vec<String>,
}

pub fn parse_open_sources<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .skip(1)
        .filter_map(|arg| normalize_open_source(arg.as_ref()))
        .collect()
}

pub fn emit_open_sources<R: Runtime>(
    app: &AppHandle<R>,
    sources: Vec<String>,
) -> tauri::Result<()> {
    app.emit(SYSTEM_OPEN_EVENT, OpenRequestPayload { sources })
}

#[cfg(windows)]
pub fn register_torrent_file_association() -> Result<(), Box<dyn Error>> {
    use windows_registry::CURRENT_USER;

    let exe = std::env::current_exe()?.display().to_string();
    let command = format!("\"{exe}\" \"%1\"");

    let extension = CURRENT_USER.create("Software\\Classes\\.torrent")?;
    extension.set_string("", "UniDL.Torrent")?;
    extension.set_string("Content Type", "application/x-bittorrent")?;

    let open_with = CURRENT_USER.create("Software\\Classes\\.torrent\\OpenWithProgids")?;
    open_with.set_string("UniDL.Torrent", "")?;

    let prog_id = CURRENT_USER.create("Software\\Classes\\UniDL.Torrent")?;
    prog_id.set_string("", "BitTorrent 文件")?;

    let icon = CURRENT_USER.create("Software\\Classes\\UniDL.Torrent\\DefaultIcon")?;
    icon.set_string("", format!("{exe},0"))?;

    let open_command =
        CURRENT_USER.create("Software\\Classes\\UniDL.Torrent\\shell\\open\\command")?;
    open_command.set_string("", command)?;

    Ok(())
}

#[cfg(not(windows))]
pub fn register_torrent_file_association() -> Result<(), Box<dyn Error>> {
    Ok(())
}

fn normalize_open_source(value: &str) -> Option<String> {
    let source = value.trim().trim_matches('"');
    if source.is_empty() {
        return None;
    }

    let lower = source.to_ascii_lowercase();
    if lower.starts_with("magnet:") || is_torrent_path(&lower) {
        return Some(source.to_string());
    }

    None
}

fn is_torrent_path(value: &str) -> bool {
    let clean = value.split(['?', '#']).next().unwrap_or(value);
    clean.ends_with(".torrent")
}
