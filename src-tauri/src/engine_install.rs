use std::{
    env,
    error::Error,
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use reqwest::blocking::Client;
use serde::Deserialize;
use zip::ZipArchive;

use crate::models::EngineKind;

const YTDLP_RELEASE_URL: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const ARIA2_RELEASE_URL: &str = "https://api.github.com/repos/aria2/aria2/releases/latest";

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub struct InstalledEngine {
    pub executable_path: PathBuf,
    pub version: String,
}

pub fn install_latest(engine: EngineKind) -> Result<InstalledEngine, Box<dyn Error>> {
    match engine {
        EngineKind::Aria2 => install_aria2(),
        EngineKind::YtDlp => install_ytdlp(),
        EngineKind::QBittorrent => Err("qBittorrent does not have a managed executable".into()),
    }
}

fn install_ytdlp() -> Result<InstalledEngine, Box<dyn Error>> {
    let client = github_client()?;
    let release = latest_release(&client, YTDLP_RELEASE_URL)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == "yt-dlp.exe")
        .ok_or("yt-dlp.exe release asset not found")?;
    let executable_path = engines_dir().join("yt-dlp.exe");
    fs::create_dir_all(parent_dir(&executable_path)?)?;
    download_to_file(&client, &asset.browser_download_url, &executable_path)?;

    Ok(InstalledEngine {
        executable_path,
        version: release.tag_name,
    })
}

fn install_aria2() -> Result<InstalledEngine, Box<dyn Error>> {
    let client = github_client()?;
    let release = latest_release(&client, ARIA2_RELEASE_URL)?;
    let asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.contains("-win-64bit-")
                && asset.name.ends_with(".zip")
                && !asset.name.contains("android")
        })
        .ok_or("aria2 Windows 64-bit release asset not found")?;
    let bytes = client
        .get(&asset.browser_download_url)
        .send()?
        .error_for_status()?
        .bytes()?;
    let executable_path = engines_dir().join("aria2c.exe");
    fs::create_dir_all(parent_dir(&executable_path)?)?;
    extract_file_from_zip(bytes.as_ref(), "aria2c.exe", &executable_path)?;

    Ok(InstalledEngine {
        executable_path,
        version: release.tag_name,
    })
}

fn github_client() -> Result<Client, Box<dyn Error>> {
    Ok(Client::builder().user_agent("UniDL").build()?)
}

fn latest_release(client: &Client, url: &str) -> Result<GitHubRelease, Box<dyn Error>> {
    Ok(client.get(url).send()?.error_for_status()?.json()?)
}

fn download_to_file(client: &Client, url: &str, path: &Path) -> Result<(), Box<dyn Error>> {
    let mut response = client.get(url).send()?.error_for_status()?;
    let tmp = path.with_extension("tmp");
    let mut file = File::create(&tmp)?;
    io::copy(&mut response, &mut file)?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn extract_file_from_zip(bytes: &[u8], file_name: &str, output: &Path) -> Result<(), Box<dyn Error>> {
    let reader = Cursor::new(bytes);
    let mut archive = ZipArchive::new(reader)?;
    let index = find_zip_index(&mut archive, file_name)?;
    let mut source = archive.by_index(index)?;
    let tmp = output.with_extension("tmp");
    let mut target = File::create(&tmp)?;
    io::copy(&mut source, &mut target)?;
    drop(source);
    if output.exists() {
        fs::remove_file(output)?;
    }
    fs::rename(tmp, output)?;
    Ok(())
}

fn find_zip_index<R: Read + io::Seek>(
    archive: &mut ZipArchive<R>,
    file_name: &str,
) -> Result<usize, Box<dyn Error>> {
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let name = entry.name().replace('\\', "/");
        if name.rsplit('/').next() == Some(file_name) {
            drop(entry);
            return Ok(index);
        }
    }

    Err(format!("{file_name} not found in release archive").into())
}

fn engines_dir() -> PathBuf {
    env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::current_dir().expect("current directory should be available"))
        .join("UniDL")
        .join("engines")
}

fn parent_dir(path: &Path) -> Result<&Path, Box<dyn Error>> {
    path.parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()).into())
}
