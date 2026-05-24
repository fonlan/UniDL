use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

use chrono::Local;

struct Logger {
    directory: PathBuf,
    current_date: String,
    file: File,
}

static LOGGER: OnceLock<Mutex<Logger>> = OnceLock::new();

pub fn init() -> Result<(), String> {
    let app_data = env::var_os("APPDATA").ok_or("APPDATA environment variable is not set")?;
    let directory = PathBuf::from(app_data).join("UniDL").join("logs");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;

    let current_date = current_date();
    let file = open_log_file(&directory, &current_date)?;
    LOGGER
        .set(Mutex::new(Logger {
            directory,
            current_date,
            file,
        }))
        .map_err(|_| "logger has already been initialized".to_string())?;

    info("logger initialized");
    Ok(())
}

pub fn info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}

pub fn write(level: &str, message: &str) {
    if let Some(logger) = LOGGER.get() {
        if let Ok(mut logger) = logger.lock() {
            if let Err(error) = logger.write(level, message) {
                eprintln!("failed to write UniDL log: {error}");
            }
        }
    }
}

impl Logger {
    fn write(&mut self, level: &str, message: &str) -> Result<(), String> {
        let date = current_date();
        if date != self.current_date {
            self.file = open_log_file(&self.directory, &date)?;
            self.current_date = date;
        }

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        writeln!(self.file, "[{timestamp}] [{level}] {message}")
            .map_err(|error| error.to_string())?;
        self.file.flush().map_err(|error| error.to_string())
    }
}

fn current_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

fn open_log_file(directory: &PathBuf, date: &str) -> Result<File, String> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(format!("{date}.log")))
        .map_err(|error| error.to_string())
}
