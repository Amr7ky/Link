mod commands;
mod dependencies;
mod downloader;
mod settings;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Child;
use tokio::sync::Mutex;

pub struct AppState {
    pub active_downloads: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            active_downloads: Arc::new(Mutex::new(HashMap::new())),
        })
        .invoke_handler(tauri::generate_handler![
            commands::open_download_folder,
            commands::get_dependency_status,
            commands::check_dependency_updates,
            commands::install_ytdlp,
            commands::install_ffmpeg,
            commands::update_ytdlp,
            commands::update_ffmpeg,
            commands::install_deno,
            commands::update_deno,
            commands::remove_deno,
            commands::remove_ytdlp,
            commands::remove_ffmpeg,
            commands::fetch_metadata,
            commands::start_download,
            commands::cancel_download,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Link");
}
