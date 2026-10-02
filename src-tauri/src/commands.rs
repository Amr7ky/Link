use crate::dependencies::{self, AppDependencies};
use crate::downloader;
use crate::settings::{self, AppSettings};
use crate::AppState;
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, State, Window};

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[tauri::command]
pub async fn get_dependency_status(app: AppHandle) -> Result<AppDependencies, String> {
    dependencies::get_status(&app).await
}

#[tauri::command]
pub async fn check_dependency_updates(app: AppHandle) -> Result<AppDependencies, String> {
    let dependencies = dependencies::check_all_updates(&app).await?;

    let mut current_settings = settings::load_settings(&app);
    current_settings.last_update_check = Some(now_unix());
    settings::save_settings(&app, current_settings)?;

    Ok(dependencies)
}

#[tauri::command]
pub async fn install_ytdlp(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_ytdlp(&app).await
}

#[tauri::command]
pub async fn install_ffmpeg(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_ffmpeg(&app).await
}

#[tauri::command]
pub async fn update_ytdlp(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_ytdlp(&app).await
}

#[tauri::command]
pub async fn update_ffmpeg(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_ffmpeg(&app).await
}

#[tauri::command]
pub async fn install_deno(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_deno(&app).await
}

#[tauri::command]
pub async fn update_deno(app: AppHandle) -> Result<(), String> {
    dependencies::install_or_update_deno(&app).await
}

#[tauri::command]
pub fn remove_deno(app: AppHandle) -> Result<(), String> {
    dependencies::remove_managed_deno(&app)
}

#[tauri::command]
pub fn remove_ytdlp(app: AppHandle) -> Result<(), String> {
    dependencies::remove_managed_ytdlp(&app)
}

#[tauri::command]
pub fn remove_ffmpeg(app: AppHandle) -> Result<(), String> {
    dependencies::remove_managed_ffmpeg(&app)
}

#[tauri::command]
pub async fn fetch_metadata(app: AppHandle, url: String) -> Result<Value, String> {
    downloader::fetch_info(&url, &app).await
}

#[tauri::command]
pub fn open_download_folder(path: String) -> Result<(), String> {
    let folder =
        std::fs::canonicalize(&path).map_err(|e| format!("Could not find download folder: {e}"))?;
    if !folder.is_dir() {
        return Err("Download path is not a folder".to_string());
    }
    std::process::Command::new("explorer.exe")
        .arg(folder)
        .spawn()
        .map_err(|e| format!("Could not open download folder: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn start_download(
    id: String,
    url: String,
    format_id: String,
    dl_type: String,
    path: String,
    has_audio: bool,
    source_ext: String,
    start_time: Option<String>,
    end_time: Option<String>,
    window: Window,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    downloader::execute_download(
        id, url, format_id, dl_type, path, has_audio, source_ext, start_time, end_time, window,
        app, state,
    )
    .await
}

#[tauri::command]
pub async fn cancel_download(id: String, state: State<'_, AppState>) -> Result<(), String> {
    if let Some(child_arc) = state.active_downloads.lock().await.remove(&id) {
        let mut child = child_arc.lock().await;
        child
            .kill()
            .await
            .map_err(|e| format!("Could not cancel download: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> AppSettings {
    settings::load_settings(&app)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, path: String) -> Result<(), String> {
    let folder =
        std::fs::canonicalize(&path).map_err(|e| format!("Could not find download folder: {e}"))?;
    if !folder.is_dir() {
        return Err("Download path is not a folder".to_string());
    }
    let mut current = settings::load_settings(&app);
    current.download_path = folder.to_string_lossy().into_owned();
    settings::save_settings(&app, current)
}
