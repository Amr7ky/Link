use futures_util::StreamExt;
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use tokio::io::AsyncWriteExt;

const USER_AGENT_VALUE: &str = "Link/1.0";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

const YTDLP_REPO: &str = "yt-dlp/yt-dlp";
const YTDLP_ASSET: &str = "yt-dlp.exe";
const YTDLP_CHECKSUMS: &str = "SHA2-256SUMS";

const FFMPEG_REPO: &str = "BtbN/FFmpeg-Builds";
const FFMPEG_PREFERRED_ASSET: &str = "ffmpeg-master-latest-win64-gpl.zip";
const FFMPEG_CHECKSUMS: &str = "checksums.sha256";
const DENO_REPO: &str = "denoland/deno";
const DENO_ASSET: &str = "deno-x86_64-pc-windows-msvc.zip";

const TOOL_YTDLP: &str = "ytdlp";
const TOOL_FFMPEG: &str = "ffmpeg";
const TOOL_DENO: &str = "deno";

#[derive(Clone, Serialize, Deserialize)]
pub struct DependencyInfo {
    pub installed: bool,
    pub install_path: String,
    pub installed_version: Option<String>,
    pub latest_version: Option<String>,
    pub status: String,
    pub update_available: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AppDependencies {
    pub yt_dlp: DependencyInfo,
    pub ffmpeg: DependencyInfo,
    pub js_runtime: DependencyInfo,
}

#[derive(Clone, Serialize)]
pub struct ProgressPayload {
    pub tool: String,
    pub phase: String,
    pub percentage: u8,
}

#[derive(Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Clone, Deserialize)]
struct GitHubAsset {
    id: u64,
    name: String,
    browser_download_url: String,
    updated_at: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct InstallMetadata {
    source_repo: String,
    release_tag: String,
    asset_name: String,
    asset_id: u64,
    asset_updated_at: Option<String>,
    sha256: String,
    installed_at: u64,
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn github_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .build()
        .map_err(|e| format!("Could not prepare secure connection: {e}"))
}

pub fn get_tools_dir(app: &AppHandle) -> PathBuf {
    let mut path = app
        .path()
        .app_local_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    path.push("tools");
    path
}

pub fn get_ytdlp_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("yt-dlp").join("yt-dlp.exe")
}

pub fn get_ffmpeg_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("ffmpeg").join("ffmpeg.exe")
}

pub fn get_ffprobe_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("ffmpeg").join("ffprobe.exe")
}

pub fn get_deno_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("deno").join("deno.exe")
}

fn system_runtime_paths() -> Vec<(String, PathBuf)> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            candidates.push(("deno".to_string(), directory.join("deno.exe")));
            candidates.push(("node".to_string(), directory.join("node.exe")));
        }
    }
    candidates
}

pub async fn js_runtime_arg(app: &AppHandle) -> Option<String> {
    let managed = get_deno_path(app);
    if executable_version(&managed, "--version").await.is_some() {
        return Some(format!("deno:{}", managed.to_string_lossy()));
    }
    for (kind, path) in system_runtime_paths() {
        let Some(version) = executable_version(&path, "--version").await else {
            continue;
        };
        let minimum = if kind == "node" { 22 } else { 2 };
        let version_number = if kind == "deno" {
            version.split_whitespace().nth(1).unwrap_or_default()
        } else {
            version.as_str()
        };
        let mut version_parts = version_number.trim_start_matches('v').split('.');
        let major = version_parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let minor = version_parts
            .next()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        if major > minimum || (major == minimum && (kind == "node" || minor >= 3)) {
            return Some(format!("{kind}:{}", path.to_string_lossy()));
        }
    }
    None
}

fn ytdlp_metadata_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("yt-dlp").join("install.json")
}

fn ffmpeg_metadata_path(app: &AppHandle) -> PathBuf {
    get_tools_dir(app).join("ffmpeg").join("install.json")
}

fn emit_progress(app: &AppHandle, tool: &str, phase: impl Into<String>, percentage: u8) {
    let _ = app.emit(
        "dependency-progress",
        ProgressPayload {
            tool: tool.to_string(),
            phase: phase.into(),
            percentage,
        },
    );
}

async fn executable_version(path: &Path, arg: &str) -> Option<String> {
    if !path.exists() {
        return None;
    }

    let mut command = tokio::process::Command::new(path);
    command.arg(arg);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|text| text.lines().next().unwrap_or_default().trim().to_string())
        .filter(|text| !text.is_empty())
}

async fn validate_executable(path: &Path, arg: &str, label: &str) -> Result<String, String> {
    let mut command = tokio::process::Command::new(path);
    command.arg(arg);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|e| format!("Could not start {label}: {e}"))?;

    if !output.status.success() {
        return Err(format!("Downloaded {label} failed its validation check"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let first_line = stdout.lines().next().unwrap_or_default().trim().to_string();
    if first_line.is_empty() {
        return Err(format!(
            "Downloaded {label} returned no version information"
        ));
    }

    Ok(first_line)
}

pub async fn get_status(app: &AppHandle) -> Result<AppDependencies, String> {
    let ytdlp_path = get_ytdlp_path(app);
    let ffmpeg_path = get_ffmpeg_path(app);
    let ffprobe_path = get_ffprobe_path(app);

    let yt_dlp_version = if ytdlp_path.exists() {
        executable_version(&ytdlp_path, "--version").await
    } else {
        None
    };
    let yt_dlp_installed = yt_dlp_version.is_some();

    let ffmpeg_line = if ffmpeg_path.exists() {
        executable_version(&ffmpeg_path, "-version").await
    } else {
        None
    };
    let ffprobe_line = if ffprobe_path.exists() {
        executable_version(&ffprobe_path, "-version").await
    } else {
        None
    };
    let ffmpeg_installed = ffmpeg_line.is_some() && ffprobe_line.is_some();
    let ffmpeg_version =
        ffmpeg_line.and_then(|line| line.split_whitespace().nth(2).map(str::to_string));
    let managed_deno = get_deno_path(app);
    let managed_deno_version = executable_version(&managed_deno, "--version").await;
    let managed_deno_installed = managed_deno_version.is_some();
    let runtime_arg = js_runtime_arg(app).await;
    let runtime_path = runtime_arg
        .as_deref()
        .and_then(|value| value.split_once(':'))
        .map(|(_, path)| path.to_string());
    let runtime_version = if let Some(version) = managed_deno_version {
        Some(version)
    } else if let Some(path) = runtime_path.as_deref() {
        executable_version(Path::new(path), "--version").await
    } else {
        None
    };

    Ok(AppDependencies {
        yt_dlp: DependencyInfo {
            installed: yt_dlp_installed,
            install_path: ytdlp_path.to_string_lossy().into_owned(),
            installed_version: yt_dlp_version,
            latest_version: None,
            status: if yt_dlp_installed {
                "Installed".to_string()
            } else {
                "Not installed".to_string()
            },
            update_available: false,
        },
        ffmpeg: DependencyInfo {
            installed: ffmpeg_installed,
            install_path: ffmpeg_path.to_string_lossy().into_owned(),
            installed_version: ffmpeg_version,
            latest_version: None,
            status: if ffmpeg_installed {
                "Installed".to_string()
            } else {
                "Not installed".to_string()
            },
            update_available: false,
        },
        js_runtime: DependencyInfo {
            installed: runtime_arg.is_some(),
            install_path: runtime_path
                .unwrap_or_else(|| managed_deno.to_string_lossy().into_owned()),
            installed_version: runtime_version,
            latest_version: None,
            status: if runtime_arg.is_none() {
                "Not installed".to_string()
            } else if managed_deno_installed {
                "Installed".to_string()
            } else {
                "System runtime".to_string()
            },
            update_available: false,
        },
    })
}

async fn fetch_latest_release(repo: &str) -> Result<GitHubRelease, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    github_client()?
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, GITHUB_ACCEPT)
        .send()
        .await
        .map_err(|e| format!("Could not contact GitHub for {repo}: {e}"))?
        .error_for_status()
        .map_err(|e| format!("GitHub returned an error for {repo}: {e}"))?
        .json::<GitHubRelease>()
        .await
        .map_err(|e| format!("Could not read GitHub release data for {repo}: {e}"))
}

async fn download_text(url: &str) -> Result<String, String> {
    github_client()?
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download server returned an error: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Could not read downloaded text: {e}"))
}

async fn download_file_with_progress(
    url: &str,
    dest: &Path,
    app: &AppHandle,
    tool: &str,
    label: &str,
) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Could not create download directory: {e}"))?;
    }

    let _ = tokio::fs::remove_file(dest).await;

    let response = github_client()?
        .get(url)
        .header(USER_AGENT, USER_AGENT_VALUE)
        .header(ACCEPT, "application/octet-stream")
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Download server returned an error: {e}"))?;

    let total_size = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest)
        .await
        .map_err(|e| format!("Could not create temporary file: {e}"))?;

    let mut downloaded = 0_u64;
    let mut stream = response.bytes_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("Download interrupted: {e}"))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| format!("Disk write error: {e}"))?;
        downloaded += chunk.len() as u64;

        let percentage = if total_size == 0 {
            0
        } else {
            ((downloaded as f64 / total_size as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8
        };

        emit_progress(app, tool, format!("Downloading {label}…"), percentage);
    }

    file.flush()
        .await
        .map_err(|e| format!("Could not finish writing download: {e}"))?;

    Ok(())
}

fn calculate_sha256(file_path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(file_path)
        .map_err(|e| format!("Unable to open file for verification: {e}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn verify_file_sha256(file_path: &Path, expected_hash: &str) -> Result<(), String> {
    let actual_hash = calculate_sha256(file_path)?;
    if actual_hash.eq_ignore_ascii_case(expected_hash.trim()) {
        Ok(())
    } else {
        Err(format!(
            "Checksum verification failed. Expected {}, got {}",
            expected_hash.trim(),
            actual_hash
        ))
    }
}

fn checksum_for_file(checksum_text: &str, filename: &str) -> Result<String, String> {
    for line in checksum_text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        let normalized = name.trim_start_matches('*');

        if normalized == filename {
            return Ok(hash.to_string());
        }
    }

    Err(format!("Checksum for {filename} was not found"))
}

fn read_metadata(path: &Path) -> Option<InstallMetadata> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<InstallMetadata>(&text).ok())
}

fn write_metadata(path: &Path, metadata: &InstallMetadata) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let temp = path.with_extension("json.tmp");
    let content = serde_json::to_vec_pretty(metadata).map_err(|e| e.to_string())?;
    fs::write(&temp, content).map_err(|e| e.to_string())?;

    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    fs::rename(&temp, path).map_err(|e| e.to_string())
}

fn safe_replace_file(new_file: &Path, final_file: &Path) -> Result<(), String> {
    let backup_file = final_file.with_extension("exe.backup");

    if backup_file.exists() {
        let _ = fs::remove_file(&backup_file);
    }

    if final_file.exists() {
        fs::rename(final_file, &backup_file).map_err(|e| {
            format!(
                "Could not prepare the existing installation for update. It may still be in use: {e}"
            )
        })?;
    }

    match fs::rename(new_file, final_file) {
        Ok(()) => {
            if backup_file.exists() {
                let _ = fs::remove_file(&backup_file);
            }
            Ok(())
        }
        Err(error) => {
            if backup_file.exists() && !final_file.exists() {
                let _ = fs::rename(&backup_file, final_file);
            }
            Err(format!("Could not activate the new executable: {error}"))
        }
    }
}

fn recover_directory_backup(target_dir: &Path, backup_dir: &Path) -> Result<(), String> {
    if !target_dir.exists() && backup_dir.exists() {
        fs::rename(backup_dir, target_dir)
            .map_err(|e| format!("Could not restore previous FFmpeg installation: {e}"))?;
    }
    Ok(())
}

fn safe_replace_directory(
    staging_dir: &Path,
    target_dir: &Path,
    backup_dir: &Path,
) -> Result<(), String> {
    recover_directory_backup(target_dir, backup_dir)?;

    if backup_dir.exists() {
        fs::remove_dir_all(backup_dir)
            .map_err(|e| format!("Could not clear stale FFmpeg backup: {e}"))?;
    }

    if target_dir.exists() {
        fs::rename(target_dir, backup_dir).map_err(|e| {
            format!(
                "Could not prepare the current FFmpeg installation for update. It may still be in use: {e}"
            )
        })?;
    }

    match fs::rename(staging_dir, target_dir) {
        Ok(()) => {
            if backup_dir.exists() {
                let _ = fs::remove_dir_all(backup_dir);
            }
            Ok(())
        }
        Err(error) => {
            if target_dir.exists() {
                let _ = fs::remove_dir_all(target_dir);
            }
            if backup_dir.exists() {
                let _ = fs::rename(backup_dir, target_dir);
            }
            Err(format!("Could not activate the new FFmpeg build: {error}"))
        }
    }
}

fn select_ffmpeg_asset(release: &GitHubRelease) -> Result<GitHubAsset, String> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == FFMPEG_PREFERRED_ASSET)
        .or_else(|| {
            release.assets.iter().find(|asset| {
                asset.name.ends_with("-win64-gpl.zip") && !asset.name.contains("-shared")
            })
        })
        .cloned()
        .ok_or_else(|| "Compatible Windows x64 static FFmpeg build was not found".to_string())
}

async fn fetch_release_checksum(
    release: &GitHubRelease,
    checksum_asset_name: &str,
    target_asset_name: &str,
) -> Result<String, String> {
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == checksum_asset_name)
        .ok_or_else(|| format!("{checksum_asset_name} was not found in the release"))?;

    let checksum_text = download_text(&checksum_asset.browser_download_url).await?;
    checksum_for_file(&checksum_text, target_asset_name)
}

pub async fn install_or_update_ytdlp(app: &AppHandle) -> Result<(), String> {
    emit_progress(app, TOOL_YTDLP, "Checking official yt-dlp release…", 0);

    let release = fetch_latest_release(YTDLP_REPO).await?;
    let exe_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == YTDLP_ASSET)
        .cloned()
        .ok_or_else(|| "Official yt-dlp.exe release asset was not found".to_string())?;

    let expected_hash = fetch_release_checksum(&release, YTDLP_CHECKSUMS, YTDLP_ASSET).await?;

    let target_dir = get_tools_dir(app).join("yt-dlp");
    fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;

    let new_file = target_dir.join("yt-dlp.new.exe");
    let final_file = target_dir.join("yt-dlp.exe");
    let _ = fs::remove_file(&new_file);

    if let Err(error) = download_file_with_progress(
        &exe_asset.browser_download_url,
        &new_file,
        app,
        TOOL_YTDLP,
        "yt-dlp",
    )
    .await
    {
        let _ = fs::remove_file(&new_file);
        return Err(error);
    }

    emit_progress(app, TOOL_YTDLP, "Verifying yt-dlp…", 100);
    if let Err(error) = verify_file_sha256(&new_file, &expected_hash) {
        let _ = fs::remove_file(&new_file);
        return Err(error);
    }

    emit_progress(app, TOOL_YTDLP, "Validating yt-dlp…", 100);
    if let Err(error) = validate_executable(&new_file, "--version", "yt-dlp").await {
        let _ = fs::remove_file(&new_file);
        return Err(error);
    }

    emit_progress(app, TOOL_YTDLP, "Installing yt-dlp…", 100);
    safe_replace_file(&new_file, &final_file)?;

    let metadata = InstallMetadata {
        source_repo: YTDLP_REPO.to_string(),
        release_tag: release.tag_name,
        asset_name: exe_asset.name,
        asset_id: exe_asset.id,
        asset_updated_at: exe_asset.updated_at,
        sha256: expected_hash,
        installed_at: now_unix(),
    };

    let _ = write_metadata(&ytdlp_metadata_path(app), &metadata);

    emit_progress(app, TOOL_YTDLP, "Installed", 100);
    Ok(())
}

fn extract_ffmpeg_archive(zip_path: &Path, staging_dir: &Path) -> Result<(), String> {
    if staging_dir.exists() {
        fs::remove_dir_all(staging_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(staging_dir).map_err(|e| e.to_string())?;

    let archive_file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(archive_file).map_err(|e| e.to_string())?;

    let mut found_ffmpeg = false;
    let mut found_ffprobe = false;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(enclosed) = entry.enclosed_name().map(|path| path.to_owned()) else {
            continue;
        };
        let Some(filename) = enclosed.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if filename.eq_ignore_ascii_case("ffmpeg.exe") {
            let mut output =
                fs::File::create(staging_dir.join("ffmpeg.exe")).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
            found_ffmpeg = true;
        } else if filename.eq_ignore_ascii_case("ffprobe.exe") {
            let mut output =
                fs::File::create(staging_dir.join("ffprobe.exe")).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
            found_ffprobe = true;
        }
    }

    if !found_ffmpeg || !found_ffprobe {
        return Err("Downloaded archive did not contain ffmpeg.exe and ffprobe.exe".to_string());
    }

    Ok(())
}

pub async fn install_or_update_ffmpeg(app: &AppHandle) -> Result<(), String> {
    emit_progress(app, TOOL_FFMPEG, "Checking trusted FFmpeg build…", 0);

    let release = fetch_latest_release(FFMPEG_REPO).await?;
    let zip_asset = select_ffmpeg_asset(&release)?;
    let expected_hash = fetch_release_checksum(&release, FFMPEG_CHECKSUMS, &zip_asset.name).await?;

    let tools_dir = get_tools_dir(app);
    fs::create_dir_all(&tools_dir).map_err(|e| e.to_string())?;

    let target_dir = tools_dir.join("ffmpeg");
    let staging_dir = tools_dir.join("ffmpeg-staging");
    let backup_dir = tools_dir.join("ffmpeg-backup");
    let temp_zip = tools_dir.join("ffmpeg.download.zip");

    recover_directory_backup(&target_dir, &backup_dir)?;
    let _ = fs::remove_dir_all(&staging_dir);
    let _ = fs::remove_file(&temp_zip);

    if let Err(error) = download_file_with_progress(
        &zip_asset.browser_download_url,
        &temp_zip,
        app,
        TOOL_FFMPEG,
        "FFmpeg",
    )
    .await
    {
        let _ = fs::remove_file(&temp_zip);
        return Err(error);
    }

    emit_progress(app, TOOL_FFMPEG, "Verifying FFmpeg…", 100);
    if let Err(error) = verify_file_sha256(&temp_zip, &expected_hash) {
        let _ = fs::remove_file(&temp_zip);
        return Err(error);
    }

    emit_progress(app, TOOL_FFMPEG, "Extracting FFmpeg…", 100);
    let zip_for_extract = temp_zip.clone();
    let staging_for_extract = staging_dir.clone();
    tokio::task::spawn_blocking(move || {
        extract_ffmpeg_archive(&zip_for_extract, &staging_for_extract)
    })
    .await
    .map_err(|e| e.to_string())??;

    emit_progress(app, TOOL_FFMPEG, "Validating FFmpeg…", 100);
    validate_executable(&staging_dir.join("ffmpeg.exe"), "-version", "FFmpeg").await?;
    validate_executable(&staging_dir.join("ffprobe.exe"), "-version", "ffprobe").await?;

    let metadata = InstallMetadata {
        source_repo: FFMPEG_REPO.to_string(),
        release_tag: release.tag_name,
        asset_name: zip_asset.name,
        asset_id: zip_asset.id,
        asset_updated_at: zip_asset.updated_at,
        sha256: expected_hash,
        installed_at: now_unix(),
    };
    write_metadata(&staging_dir.join("install.json"), &metadata)?;

    emit_progress(app, TOOL_FFMPEG, "Installing FFmpeg…", 100);
    if let Err(error) = safe_replace_directory(&staging_dir, &target_dir, &backup_dir) {
        let _ = fs::remove_file(&temp_zip);
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    let _ = fs::remove_file(&temp_zip);
    emit_progress(app, TOOL_FFMPEG, "Installed", 100);
    Ok(())
}

fn deno_checksum(text: &str) -> Result<String, String> {
    text.lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("Hash"))
        .map(|(_, hash)| hash.trim().to_string())
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "Deno release checksum was invalid".to_string())
}

fn extract_deno_archive(zip_path: &Path, staging_dir: &Path) -> Result<(), String> {
    if staging_dir.exists() {
        fs::remove_dir_all(staging_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(staging_dir).map_err(|e| e.to_string())?;
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        if path.file_name().and_then(|name| name.to_str()) == Some("deno.exe") {
            let mut output =
                fs::File::create(staging_dir.join("deno.exe")).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut output).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err("Deno archive did not contain deno.exe".to_string())
}

pub async fn install_or_update_deno(app: &AppHandle) -> Result<(), String> {
    emit_progress(app, TOOL_DENO, "Checking official Deno release…", 0);
    let release = fetch_latest_release(DENO_REPO).await?;
    let zip_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == DENO_ASSET)
        .ok_or("Windows x64 Deno release was not found")?;
    let checksum_name = format!("{DENO_ASSET}.sha256sum");
    let checksum_asset = release
        .assets
        .iter()
        .find(|asset| asset.name == checksum_name)
        .ok_or("Deno release checksum was not found")?;
    let checksum_text = download_text(&checksum_asset.browser_download_url).await?;
    let expected_hash = deno_checksum(&checksum_text)?;

    let tools_dir = get_tools_dir(app);
    fs::create_dir_all(&tools_dir).map_err(|e| e.to_string())?;
    let zip_path = tools_dir.join("deno.download.zip");
    let staging = tools_dir.join("deno-staging");
    let target = tools_dir.join("deno");
    let backup = tools_dir.join("deno-backup");
    recover_directory_backup(&target, &backup)?;
    download_file_with_progress(
        &zip_asset.browser_download_url,
        &zip_path,
        app,
        TOOL_DENO,
        "Deno",
    )
    .await?;
    emit_progress(app, TOOL_DENO, "Verifying Deno…", 100);
    verify_file_sha256(&zip_path, &expected_hash)?;
    let extract_zip = zip_path.clone();
    let extract_stage = staging.clone();
    tokio::task::spawn_blocking(move || extract_deno_archive(&extract_zip, &extract_stage))
        .await
        .map_err(|e| e.to_string())??;
    validate_executable(&staging.join("deno.exe"), "--version", "Deno").await?;
    let metadata = InstallMetadata {
        source_repo: DENO_REPO.to_string(),
        release_tag: release.tag_name,
        asset_name: zip_asset.name.clone(),
        asset_id: zip_asset.id,
        asset_updated_at: zip_asset.updated_at.clone(),
        sha256: expected_hash,
        installed_at: now_unix(),
    };
    write_metadata(&staging.join("install.json"), &metadata)?;
    safe_replace_directory(&staging, &target, &backup)?;
    let _ = fs::remove_file(zip_path);
    emit_progress(app, TOOL_DENO, "Installed", 100);
    Ok(())
}

pub fn remove_managed_deno(app: &AppHandle) -> Result<(), String> {
    let directory = get_tools_dir(app).join("deno");
    if directory.exists() {
        fs::remove_dir_all(directory).map_err(|e| format!("Failed to delete Deno: {e}"))?;
    }
    Ok(())
}

pub fn remove_managed_ytdlp(app: &AppHandle) -> Result<(), String> {
    let dir = get_tools_dir(app).join("yt-dlp");
    if dir.exists() {
        fs::remove_dir_all(&dir).map_err(|e| format!("Failed to delete yt-dlp: {e}"))?;
    }
    Ok(())
}

pub fn remove_managed_ffmpeg(app: &AppHandle) -> Result<(), String> {
    let tools_dir = get_tools_dir(app);
    for dir in [
        tools_dir.join("ffmpeg"),
        tools_dir.join("ffmpeg-staging"),
        tools_dir.join("ffmpeg-backup"),
    ] {
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|e| format!("Failed to delete managed FFmpeg files: {e}"))?;
        }
    }
    let temp_zip = tools_dir.join("ffmpeg.download.zip");
    if temp_zip.exists() {
        let _ = fs::remove_file(temp_zip);
    }
    Ok(())
}

pub async fn check_all_updates(app: &AppHandle) -> Result<AppDependencies, String> {
    let mut status = get_status(app).await?;

    let yt_release = fetch_latest_release(YTDLP_REPO).await?;
    let latest_yt = yt_release.tag_name.trim_start_matches('v').to_string();
    status.yt_dlp.latest_version = Some(latest_yt.clone());

    if status.yt_dlp.installed {
        let installed = status
            .yt_dlp
            .installed_version
            .clone()
            .unwrap_or_default()
            .trim_start_matches('v')
            .to_string();
        status.yt_dlp.update_available = installed.is_empty() || installed != latest_yt;
        status.yt_dlp.status = if status.yt_dlp.update_available {
            "Update available".to_string()
        } else {
            "Up to date".to_string()
        };
    }

    let ff_release = fetch_latest_release(FFMPEG_REPO).await?;
    let ff_asset = select_ffmpeg_asset(&ff_release)?;
    let latest_ff_hash =
        fetch_release_checksum(&ff_release, FFMPEG_CHECKSUMS, &ff_asset.name).await?;

    status.ffmpeg.latest_version = ff_asset
        .updated_at
        .clone()
        .or(ff_release.published_at.clone())
        .or_else(|| Some(ff_release.tag_name.clone()));

    if status.ffmpeg.installed {
        let installed_meta = read_metadata(&ffmpeg_metadata_path(app));
        status.ffmpeg.update_available = installed_meta
            .as_ref()
            .map(|meta| !meta.sha256.eq_ignore_ascii_case(&latest_ff_hash))
            .unwrap_or(true);

        status.ffmpeg.status = if status.ffmpeg.update_available {
            "Update available".to_string()
        } else {
            "Up to date".to_string()
        };
    }

    if get_deno_path(app).is_file() {
        let release = fetch_latest_release(DENO_REPO).await?;
        let latest = release.tag_name.trim_start_matches('v').to_string();
        status.js_runtime.latest_version = Some(latest.clone());
        let installed = status
            .js_runtime
            .installed_version
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .nth(1)
            .unwrap_or_default();
        status.js_runtime.update_available = installed != latest;
        status.js_runtime.status = if status.js_runtime.update_available {
            "Update available".to_string()
        } else {
            "Up to date".to_string()
        };
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::deno_checksum;

    #[test]
    fn reads_windows_release_checksum() {
        let hash = "7FDD1F42E6B0855421ECF27BB406E2492ADE1087C85E30EBF0DEAB6280EA743C";
        let text = format!("Algorithm : SHA256\nHash : {hash}\nPath : C:\\deno.zip\n");
        assert_eq!(deno_checksum(&text).unwrap(), hash);
    }
}
