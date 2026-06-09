use crate::models;

use tauri::{AppHandle, Runtime};

#[cfg(windows)]
mod platform {
    use super::*;
    use tauri_winrt_notification::Toast;
    use windows_registry::CURRENT_USER;

    pub fn register_app_identity<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
        let config = app.config();
        let product_name = config
            .product_name
            .as_deref()
            .ok_or_else(|| "app productName is required for Windows notifications".to_string())?;
        let key = CURRENT_USER
            .create(format!(
                r"SOFTWARE\Classes\AppUserModelId\{}",
                config.identifier
            ))
            .map_err(|error| error.to_string())?;
        key.set_string("DisplayName", product_name)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn show_download_completed<R: Runtime>(
        app: &AppHandle<R>,
        task: &models::DownloadTask,
    ) -> Result<(), String> {
        Toast::new(&app.config().identifier)
            .title("下载完成")
            .text1(&format!("{} 已下载完成", task.file_name))
            .show()
            .map_err(|error| error.to_string())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use tauri_plugin_notification::NotificationExt;

    pub fn register_app_identity<R: Runtime>(_app: &AppHandle<R>) -> Result<(), String> {
        Ok(())
    }

    pub fn show_download_completed<R: Runtime>(
        app: &AppHandle<R>,
        task: &models::DownloadTask,
    ) -> Result<(), String> {
        app.notification()
            .builder()
            .title("下载完成")
            .body(format!("{} 已下载完成", task.file_name))
            .show()
            .map_err(|error| error.to_string())
    }
}

pub fn register_app_identity<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    platform::register_app_identity(app)
}

pub fn show_download_completed<R: Runtime>(
    app: &AppHandle<R>,
    task: &models::DownloadTask,
) -> Result<(), String> {
    platform::show_download_completed(app, task)
}
