mod commands;
mod dependencies;
mod downloader;
mod settings;

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use tokio::process::Child;
use tokio::sync::Mutex;

pub const DOWNLOAD_RUNNING: u8 = 0;
pub const DOWNLOAD_PAUSED: u8 = 1;
pub const DOWNLOAD_CANCELLED: u8 = 2;

pub struct DownloadProcess {
    pub child: Mutex<Child>,
    state: AtomicU8,
}

impl DownloadProcess {
    pub fn new(child: Child) -> Self {
        Self {
            child: Mutex::new(child),
            state: AtomicU8::new(DOWNLOAD_RUNNING),
        }
    }

    pub fn state(&self) -> u8 {
        self.state.load(Ordering::SeqCst)
    }

    pub fn set_state(&self, state: u8) {
        self.state.store(state, Ordering::SeqCst);
    }
}

pub struct AppState {
    pub active_downloads: Arc<Mutex<HashMap<String, Arc<DownloadProcess>>>>,
}

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
            commands::pause_download,
            commands::cancel_download,
            commands::get_settings,
            commands::save_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Link");
}
