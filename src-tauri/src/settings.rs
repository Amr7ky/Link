use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone)]
pub struct AppSettings {
    pub download_path: String,
    #[serde(default)]
    pub last_update_check: Option<u64>,
}

fn get_settings_path(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_config_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    let _ = fs::create_dir_all(&path);
    path.push("settings.json");
    path
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    let path = get_settings_path(app);

    if let Ok(content) = fs::read_to_string(path) {
        if let Ok(settings) = serde_json::from_str::<AppSettings>(&content) {
            return settings;
        }
    }

    let default_path = app
        .path()
        .download_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|_| String::from("C:\\Downloads"));

    AppSettings {
        download_path: default_path,
        last_update_check: None,
    }
}

pub fn save_settings(app: &AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path(app);
    let content = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(path, content).map_err(|e| e.to_string())
}
