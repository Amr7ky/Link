use crate::dependencies::{get_ffmpeg_path, get_ytdlp_path, js_runtime_arg};
use crate::AppState;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State, Window};
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::Mutex;

#[derive(Clone, Serialize)]
struct ProgressEvent {
    id: String,
    percent: f64,
    speed: String,
    status: String,
    processing: bool,
}

#[derive(Clone, Serialize)]
struct CompletionEvent {
    id: String,
    success: bool,
    error: Option<String>,
    file_size: Option<u64>,
}

fn progress_event(line: &str, id: &str) -> Option<ProgressEvent> {
    if let Some((_, text)) = line.split_once("LINKPROGRESS:") {
        let mut parts = text.splitn(2, '|');
        let percent = parts
            .next()?
            .trim()
            .trim_end_matches('%')
            .trim()
            .parse::<f64>()
            .ok()?;
        let speed = parts.next().unwrap_or_default().trim().to_string();
        return Some(ProgressEvent {
            id: id.to_string(),
            percent: percent.clamp(0.0, 100.0),
            speed,
            status: format!("Downloading {:.0}%", percent),
            processing: false,
        });
    }

    if [
        "LINKPROCESSING",
        "[Merger]",
        "[VideoConvertor]",
        "[VideoRemuxer]",
        "[ExtractAudio]",
    ]
    .iter()
    .any(|marker| line.contains(marker))
    {
        return Some(ProgressEvent {
            id: id.to_string(),
            percent: 90.0,
            speed: String::new(),
            status: "Downloading…".to_string(),
            processing: true,
        });
    }

    None
}

fn parse_clock(value: &str) -> Option<f64> {
    let mut parts = value.split(':');
    let hours = parts.next()?.parse::<f64>().ok()?;
    let minutes = parts.next()?.parse::<f64>().ok()?;
    let seconds = parts.next()?.parse::<f64>().ok()?;
    if parts.next().is_some() || minutes >= 60.0 || seconds >= 60.0 {
        return None;
    }
    Some(hours * 3600.0 + minutes * 60.0 + seconds)
}

async fn trim_download(
    input: &Path,
    ffmpeg_path: &Path,
    start: f64,
    end: f64,
    is_video: bool,
    id: &str,
    window: &Window,
    active_downloads: &Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
) -> Result<PathBuf, String> {
    let extension = input
        .extension()
        .and_then(|part| part.to_str())
        .unwrap_or("mp4");
    let trimmed = input.with_extension(format!("link-trim.{extension}"));
    let backup = input.with_extension(format!("link-original.{extension}"));
    let _ = std::fs::remove_file(&trimmed);

    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-y",
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-stats_period",
            "0.25",
            "-progress",
            "pipe:1",
            "-ss",
        ])
        .arg(format!("{start:.3}"))
        .arg("-i")
        .arg(input)
        .arg("-t")
        .arg(format!("{:.3}", end - start));
    if is_video {
        command.args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
        ]);
    } else {
        command.args(["-map", "0:a:0", "-c", "copy"]);
    }
    command
        .arg(&trimmed)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    let mut active = active_downloads.lock().await;
    if !active.contains_key(id) {
        return Err("Download cancelled".to_string());
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not trim the download: {error}"))?;
    let stdout = child.stdout.take().ok_or("Could not read trim progress")?;
    let stderr = child.stderr.take().ok_or("Could not read trim errors")?;
    let child = Arc::new(Mutex::new(child));
    active.insert(id.to_string(), child.clone());
    drop(active);

    let progress_window = window.clone();
    let progress_id = id.to_string();
    let duration = end - start;
    let progress_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(value) = line.strip_prefix("out_time=") {
                if let Some(elapsed) = parse_clock(value) {
                    let percent = 90.0 + (elapsed / duration).clamp(0.0, 1.0) * 9.0;
                    let _ = progress_window.emit(
                        "download-progress",
                        ProgressEvent {
                            id: progress_id.clone(),
                            percent,
                            speed: String::new(),
                            status: "Downloading…".to_string(),
                            processing: true,
                        },
                    );
                }
            }
        }
    });
    let error_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut errors = String::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if errors.len() < 8_000 {
                errors.push_str(&line);
                errors.push('\n');
            }
        }
        errors
    });

    let status = loop {
        let result = child.lock().await.try_wait();
        match result {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => tokio::time::sleep(Duration::from_millis(120)).await,
            Err(error) => break Err(error),
        }
    };
    let _ = progress_task.await;
    let errors = error_task.await.unwrap_or_default();
    if !matches!(status, Ok(status) if status.success()) {
        let _ = std::fs::remove_file(&trimmed);
        return Err(if errors.trim().is_empty() {
            "Could not finish trimming the download".to_string()
        } else {
            errors.trim().to_string()
        });
    }

    std::fs::rename(input, &backup)
        .map_err(|error| format!("Could not prepare trimmed file: {error}"))?;
    if let Err(error) = std::fs::rename(&trimmed, input) {
        let _ = std::fs::rename(&backup, input);
        return Err(format!("Could not save trimmed file: {error}"));
    }
    let _ = std::fs::remove_file(&backup);
    Ok(input.to_owned())
}

fn find_completed_file(directory: &Path, suffix: &str, is_video: bool) -> Option<PathBuf> {
    let ending = format!(" {suffix}.{}", if is_video { "mp4" } else { "mp3" });
    std::fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(&ending))
        })
}

fn validate_media_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Enter a valid media URL".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host().is_none() {
        return Err("Only HTTP and HTTPS media URLs are supported".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Media URLs cannot contain credentials".to_string());
    }
    Ok(())
}

pub async fn fetch_info(url: &str, app: &AppHandle) -> Result<Value, String> {
    validate_media_url(url)?;
    let ytdlp_path = get_ytdlp_path(app);
    if !ytdlp_path.exists() {
        return Err("yt-dlp is not installed".to_string());
    }

    let mut command = Command::new(&ytdlp_path);
    command.args(["--ignore-config", "-J", "--no-playlist", "--no-warnings"]);
    if let Some(runtime) = js_runtime_arg(app).await {
        command.arg("--js-runtimes").arg(runtime);
    }
    command.arg("--").arg(url);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|e| format!("Failed to run yt-dlp: {e}"))?;

    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if error.is_empty() {
            "yt-dlp could not read this URL".to_string()
        } else {
            error
        });
    }

    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Invalid metadata JSON returned by yt-dlp: {e}"))
}

#[allow(clippy::too_many_arguments)]
pub async fn execute_download(
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
    if id.is_empty() || id.len() > 80 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Invalid download identifier".to_string());
    }
    validate_media_url(&url)?;
    if !matches!(dl_type.as_str(), "audio" | "video") {
        return Err("Unknown download type".to_string());
    }
    if !std::path::Path::new(&path).is_dir() {
        return Err("Download folder does not exist".to_string());
    }
    let ytdlp_path = get_ytdlp_path(&app);
    let ffmpeg_path = get_ffmpeg_path(&app);

    if !ytdlp_path.exists() {
        return Err("yt-dlp is not installed".to_string());
    }

    let trim_range = match (start_time.as_deref(), end_time.as_deref()) {
        (Some(start), Some(end)) => {
            let start = parse_clock(start).ok_or("Invalid trim start time")?;
            let end = parse_clock(end).ok_or("Invalid trim end time")?;
            if end <= start {
                return Err("Trim end must be after the start".to_string());
            }
            Some((start, end))
        }
        (None, None) => None,
        _ => return Err("Both trim times are required".to_string()),
    };
    let trimming = trim_range.is_some();
    let convert_to_mp4 = dl_type == "video" && source_ext.to_ascii_lowercase() != "mp4";
    let needs_ffmpeg = dl_type == "audio" || trimming || convert_to_mp4;

    if needs_ffmpeg && !ffmpeg_path.exists() {
        return Err("FFmpeg is required for this download but is not installed".to_string());
    }

    let job_suffix = id
        .trim_start_matches("dl-")
        .chars()
        .take(12)
        .collect::<String>();
    let download_dir = PathBuf::from(&path);
    let is_video = dl_type == "video";
    let mut args = vec![
        "--ignore-config".to_string(),
        "--newline".to_string(),
        "--progress".to_string(),
        "--progress-delta".to_string(),
        "0.2".to_string(),
        "--no-colors".to_string(),
        "--no-playlist".to_string(),
        "--no-simulate".to_string(),
        "--windows-filenames".to_string(),
        "--paths".to_string(),
        path,
        "--output".to_string(),
        format!("%(title).80s [%(id)s] {job_suffix}.%(ext)s"),
        "--progress-template".to_string(),
        "download:LINKPROGRESS:%(progress._percent_str)s|%(progress._speed_str)s".to_string(),
        "--progress-template".to_string(),
        "postprocess:LINKPROCESSING".to_string(),
        "--print".to_string(),
        "after_move:LINKFILE:%(filepath)s".to_string(),
    ];

    if ffmpeg_path.exists() {
        if let Some(parent) = ffmpeg_path.parent() {
            args.push("--ffmpeg-location".to_string());
            args.push(parent.to_string_lossy().to_string());
        }
    }

    if dl_type == "audio" {
        args.extend([
            "-f".to_string(),
            format_id,
            "--extract-audio".to_string(),
            "--audio-format".to_string(),
            "mp3".to_string(),
        ]);
    } else if has_audio {
        args.extend(["-f".to_string(), format_id]);
    } else {
        args.extend([
            "-f".to_string(),
            format!("{format_id}+bestaudio[ext=m4a]/{format_id}+bestaudio/best"),
        ]);
    }

    if dl_type == "video" && source_ext.eq_ignore_ascii_case("mp4") && !has_audio {
        args.extend(["--merge-output-format".to_string(), "mp4".to_string()]);
    }

    if convert_to_mp4 {
        args.extend(["--recode-video".to_string(), "mp4".to_string()]);
    }

    if let Some(runtime) = js_runtime_arg(&app).await {
        args.extend(["--js-runtimes".to_string(), runtime]);
    }

    args.push("--".to_string());
    args.push(url);

    let mut command = Command::new(&ytdlp_path);
    command
        .args(&args)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Could not capture yt-dlp output".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Could not capture yt-dlp errors".to_string())?;

    let child_arc = Arc::new(Mutex::new(child));
    state
        .active_downloads
        .lock()
        .await
        .insert(id.clone(), child_arc.clone());

    let active_downloads = state.active_downloads.clone();
    let id_for_task = id.clone();
    let window_for_task = window.clone();
    let expected_streams = if is_video && !has_audio { 2.0 } else { 1.0 };

    tokio::spawn(async move {
        let stdout_window = window_for_task.clone();
        let stdout_id = id_for_task.clone();
        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut buffer = Vec::new();
            let mut final_path = None;
            let mut completed_streams = 0.0;
            let mut last_stream_percent = 0.0;

            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buffer);
                        let line = line.trim_end_matches(|c| c == '\r' || c == '\n');
                        if let Some((_, path)) = line.split_once("LINKFILE:") {
                            final_path = Some(path.trim().to_string());
                        } else if let Some(mut progress) = progress_event(line, &stdout_id) {
                            if !progress.processing {
                                if last_stream_percent >= 99.0 && progress.percent < 50.0 {
                                    completed_streams += 1.0;
                                }
                                last_stream_percent = progress.percent;
                                progress.percent = ((completed_streams * 100.0 + progress.percent)
                                    / expected_streams)
                                    .clamp(0.0, 100.0);
                                progress.status = format!("Downloading {:.0}%", progress.percent);
                            }
                            let _ = stdout_window.emit("download-progress", progress);
                        }
                    }
                }
            }

            final_path
        });

        let stderr_window = window_for_task.clone();
        let stderr_id = id_for_task.clone();
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut buffer = Vec::new();
            let mut collected = String::new();

            loop {
                buffer.clear();
                match reader.read_until(b'\n', &mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let line = String::from_utf8_lossy(&buffer);
                        let line = line.trim_end_matches(|c| c == '\r' || c == '\n');
                        if let Some(progress) = progress_event(line, &stderr_id) {
                            let _ = stderr_window.emit("download-progress", progress);
                        } else if collected.len() < 16_000 {
                            collected.push_str(line);
                            collected.push('\n');
                        }
                    }
                }
            }

            collected
        });

        let exit_status = loop {
            let result = {
                let mut child = child_arc.lock().await;
                child.try_wait()
            };

            match result {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => tokio::time::sleep(Duration::from_millis(120)).await,
                Err(error) => break Err(error),
            }
        };

        let printed_path = stdout_task.await.unwrap_or_default();
        let stderr_text = stderr_task.await.unwrap_or_default();
        let result: Result<u64, String> = async {
            match exit_status {
                Ok(status) if status.success() => {}
                Ok(status) => {
                    return Err(if stderr_text.trim().is_empty() {
                        format!("yt-dlp exited with {status}")
                    } else {
                        stderr_text.trim().to_string()
                    });
                }
                Err(error) => return Err(error.to_string()),
            }

            let file = printed_path
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .or_else(|| find_completed_file(&download_dir, &job_suffix, is_video))
                .ok_or("Download finished, but the saved file could not be found")?;
            if let Some((start, end)) = trim_range {
                trim_download(
                    &file,
                    &ffmpeg_path,
                    start,
                    end,
                    is_video,
                    &id_for_task,
                    &window_for_task,
                    &active_downloads,
                )
                .await?;
            }
            std::fs::metadata(&file)
                .map(|metadata| metadata.len())
                .map_err(|error| format!("Could not read saved file size: {error}"))
        }
        .await;
        active_downloads.lock().await.remove(&id_for_task);

        let (success, error, file_size) = match result {
            Ok(size) => (true, None, Some(size)),
            Err(error) => (false, Some(error), None),
        };
        let _ = window_for_task.emit(
            "download-completed",
            CompletionEvent {
                id: id_for_task,
                success,
                error,
                file_size,
            },
        );
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_clock, progress_event};

    #[test]
    fn parses_prefixed_download_progress() {
        let event = progress_event("[download] LINKPROGRESS: 24.5%|1.2MiB/s", "job").unwrap();
        assert_eq!(event.percent, 24.5);
        assert_eq!(event.speed, "1.2MiB/s");
        assert!(!event.processing);
    }

    #[test]
    fn keeps_postprocessing_in_download_state() {
        let event = progress_event("[VideoConvertor] converting", "job").unwrap();
        assert!(event.processing);
        assert_eq!(event.status, "Downloading…");
        assert_eq!(event.percent, 90.0);
    }

    #[test]
    fn reads_trim_progress_clock() {
        assert_eq!(parse_clock("00:00:22.500000"), Some(22.5));
        assert_eq!(parse_clock("00:01:00.000000"), Some(60.0));
        assert_eq!(parse_clock("N/A"), None);
    }
}
