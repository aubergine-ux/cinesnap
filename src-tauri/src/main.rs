#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod devices;
mod recorder;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cinesnap_core::events::EventLog;
use cinesnap_core::export::{export, Cancelled, ExportOptions};
use cinesnap_core::ffmpeg::{self, Encoder};
use cinesnap_core::motion::{self, MotionTracks};
use cinesnap_core::project::{Project, Zoom, PROJECT_FILE};
use recorder::{RecordOptions, Recording};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Recording>>,
    /// Guards against two overlapping `start_recording` calls.
    starting: AtomicBool,
    encoder: Mutex<Option<Encoder>>,
    export_cancel: Arc<AtomicBool>,
    /// The event log of the project being edited (it never changes on disk).
    log: Mutex<Option<(PathBuf, Arc<EventLog>)>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    format!("{e:#}")
}

fn data_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok()
}

fn find_ffmpeg(app: &AppHandle) -> CmdResult<PathBuf> {
    ffmpeg::locate(data_dir(app).as_deref()).ok_or_else(|| "ffmpeg is not installed".to_owned())
}

fn encoder(state: &AppState, ffmpeg: &Path) -> Encoder {
    *state
        .encoder
        .lock()
        .unwrap()
        .get_or_insert_with(|| Encoder::detect(ffmpeg))
}

fn recordings_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    let base = app
        .path()
        .video_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(err)?;
    Ok(base.join("Cinesnap"))
}

fn event_log(state: &AppState, dir: &Path, project: &Project) -> Arc<EventLog> {
    let mut cache = state.log.lock().unwrap();
    if let Some((path, log)) = cache.as_ref() {
        if path == dir {
            return log.clone();
        }
    }
    let log = Arc::new(
        EventLog::load(&project.events_path(dir)).unwrap_or_else(|_| {
            EventLog::empty(project.source.video.width, project.source.video.height)
        }),
    );
    *cache = Some((dir.to_owned(), log.clone()));
    log
}

#[derive(Serialize)]
struct FfmpegInfo {
    path: String,
    version: String,
}

#[derive(Serialize)]
struct Capabilities {
    platform: &'static str,
    cursor: &'static str,
    clicks: &'static str,
    ffmpeg: Option<FfmpegInfo>,
    encoder: String,
    notes: Vec<String>,
}

#[tauri::command(async)]
fn get_capabilities(app: AppHandle, state: State<AppState>) -> Capabilities {
    let ffmpeg = find_ffmpeg(&app).ok();
    let (cursor, clicks, notes) = devices::input_capabilities();
    Capabilities {
        platform: devices::platform(),
        cursor,
        clicks,
        encoder: ffmpeg
            .as_deref()
            .map(|f| encoder(&state, f).name().to_owned())
            .unwrap_or_default(),
        ffmpeg: ffmpeg.map(|p| FfmpegInfo {
            version: ffmpeg::version(&p).unwrap_or_default(),
            path: p.display().to_string(),
        }),
        notes,
    }
}

#[tauri::command(async)]
fn download_ffmpeg(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    use ffmpeg_sidecar::download::{
        download_ffmpeg_package_with_progress, ffmpeg_download_url, unpack_ffmpeg,
        FfmpegDownloadProgressEvent,
    };
    let dir = data_dir(&app).ok_or("no data directory")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let url = ffmpeg_download_url().map_err(err)?;
    let emitter = app.clone();
    let archive = download_ffmpeg_package_with_progress(url, &dir, move |e| {
        if let FfmpegDownloadProgressEvent::Downloading {
            total_bytes,
            downloaded_bytes,
        } = e
        {
            let fraction = downloaded_bytes as f64 / total_bytes.max(1) as f64;
            let _ = emitter.emit("ffmpeg://progress", json!({ "fraction": fraction }));
        }
    })
    .map_err(err)?;
    unpack_ffmpeg(&archive, &dir).map_err(err)?;
    *state.encoder.lock().unwrap() = None;
    find_ffmpeg(&app).map(|_| ())
}

#[tauri::command(async)]
fn list_sources(app: AppHandle) -> devices::Sources {
    devices::sources(find_ffmpeg(&app).ok().as_deref())
}

#[tauri::command(async)]
fn start_recording(
    app: AppHandle,
    state: State<AppState>,
    options: RecordOptions,
) -> CmdResult<()> {
    if state.recording.lock().unwrap().is_some() || state.starting.swap(true, Ordering::SeqCst) {
        return Err("a recording is already in progress".into());
    }
    let result: CmdResult<()> = (|| {
        let ffmpeg = find_ffmpeg(&app)?;
        let enc = encoder(&state, &ffmpeg);
        let name = chrono::Local::now()
            .format("Recording %Y-%m-%d %H.%M.%S.cinesnap")
            .to_string();
        let dir = recordings_dir(&app)?.join(name);
        let recording = Recording::start(&ffmpeg, enc, &dir, &options).map_err(err)?;
        *state.recording.lock().unwrap() = Some(recording);
        Ok(())
    })();
    state.starting.store(false, Ordering::SeqCst);
    result?;

    // Get out of the picture; the user brings the window back to stop.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
    std::thread::spawn(move || loop {
        let elapsed = app
            .state::<AppState>()
            .recording
            .lock()
            .unwrap()
            .as_ref()
            .map(|r| r.elapsed_ms());
        match elapsed {
            Some(ms) => {
                let _ = app.emit("recording://tick", json!({ "elapsedMs": ms }));
            }
            None => break,
        }
        std::thread::sleep(Duration::from_millis(200));
    });
    Ok(())
}

#[tauri::command(async)]
fn stop_recording(app: AppHandle, state: State<AppState>) -> CmdResult<String> {
    let recording = state
        .recording
        .lock()
        .unwrap()
        .take()
        .ok_or("nothing is being recorded")?;
    let ffmpeg = find_ffmpeg(&app)?;
    let dir = recording.stop(&ffmpeg).map_err(err)?;
    Ok(dir.display().to_string())
}

#[tauri::command(async)]
fn cancel_recording(state: State<AppState>) {
    if let Some(recording) = state.recording.lock().unwrap().take() {
        recording.cancel();
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Recent {
    path: String,
    name: String,
    created_at: String,
    duration_ms: f64,
}

#[tauri::command(async)]
fn list_recent(app: AppHandle) -> Vec<Recent> {
    let Ok(dir) = recordings_dir(&app) else {
        return vec![];
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut list: Vec<Recent> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "cinesnap"))
        .filter_map(|p| {
            let project = Project::load(&p).ok()?;
            Some(Recent {
                name: p.file_stem()?.to_string_lossy().into_owned(),
                path: p.display().to_string(),
                created_at: project.created_at,
                duration_ms: project.source.video.duration_ms,
            })
        })
        .collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    list
}

#[tauri::command(async)]
fn open_project(path: String) -> CmdResult<Project> {
    Project::load(Path::new(&path)).map_err(err)
}

#[tauri::command(async)]
fn save_project(path: String, mut project: Project) -> CmdResult<()> {
    project.sanitize();
    project.save(Path::new(&path)).map_err(err)
}

#[tauri::command(async)]
fn delete_project(state: State<AppState>, path: String) -> CmdResult<()> {
    let dir = Path::new(&path);
    // Only ever remove something that is unmistakably a recording bundle.
    if dir.extension().is_none_or(|e| e != "cinesnap") || !dir.join(PROJECT_FILE).is_file() {
        return Err("not a Cinesnap recording".into());
    }
    state.log.lock().unwrap().take();
    std::fs::remove_dir_all(dir).map_err(err)
}

#[tauri::command(async)]
fn auto_zoom(state: State<AppState>, path: String, mut project: Project) -> Vec<Zoom> {
    project.sanitize();
    motion::auto_zoom(&project, &event_log(&state, Path::new(&path), &project))
}

#[tauri::command(async)]
fn compute_motion(state: State<AppState>, path: String, mut project: Project) -> MotionTracks {
    project.sanitize();
    motion::compute_motion(&project, &event_log(&state, Path::new(&path), &project))
}

#[tauri::command(async)]
fn export_video(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    project: Project,
    out_path: String,
) -> CmdResult<()> {
    let ffmpeg = find_ffmpeg(&app)?;
    let cancel = state.export_cancel.clone();
    cancel.store(false, Ordering::SeqCst);
    let opts = ExportOptions {
        ffmpeg: &ffmpeg,
        out: Path::new(&out_path),
        encoder: Some(encoder(&state, &ffmpeg)),
    };
    let mut last = Instant::now() - Duration::from_secs(1);
    let mut progress = |p: cinesnap_core::export::Progress| {
        if p.frame == p.total || last.elapsed() > Duration::from_millis(60) {
            last = Instant::now();
            let _ = app.emit("export://progress", p);
        }
    };
    export(Path::new(&path), &project, &opts, &mut progress, &cancel).map_err(|e| {
        if e.is::<Cancelled>() {
            "cancelled".to_owned()
        } else {
            err(e)
        }
    })
}

#[tauri::command]
fn cancel_export(state: State<AppState>) {
    state.export_cancel.store(true, Ordering::SeqCst);
}

/// WebKitGTK's DMA-BUF renderer crashes on Wayland with the proprietary
/// NVIDIA driver ("Error 71 (Protocol error) dispatching to Wayland
/// display"). Fall back to the shared-memory path there.
#[cfg(target_os = "linux")]
fn webkit_workarounds() {
    let nvidia = Path::new("/proc/driver/nvidia/version").exists();
    if nvidia && devices::is_wayland() && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

/// `cinesnap record <dir> [--seconds N] [--fps N] [--mic ID] [--webcam ID]`:
/// record without opening the window. Handy for scripts and for checking
/// that capture works on a machine.
fn headless_record(args: &[String]) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut dir = None;
    let (mut seconds, mut fps, mut mic, mut webcam) = (5.0f64, 60u32, None, None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--seconds" => seconds = value()?.parse()?,
            "--fps" => fps = value()?.parse()?,
            "--mic" => mic = Some(value()?.clone()),
            "--webcam" => webcam = Some(value()?.clone()),
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let dir = dir.context("usage: cinesnap record <dir.cinesnap> [--seconds N] [--fps N] [--mic ID] [--webcam ID]")?;
    let ffmpeg = ffmpeg::locate(None).context("ffmpeg not found")?;
    let options = RecordOptions {
        target: recorder::Target { kind: "screen".into(), id: None },
        region: None,
        mic,
        webcam,
        fps,
    };
    let recording = Recording::start(&ffmpeg, Encoder::detect(&ffmpeg), &dir, &options)?;
    eprintln!("recording for {seconds} s…");
    std::thread::sleep(Duration::from_secs_f64(seconds));
    let dir = recording.stop(&ffmpeg)?;
    println!("{}", dir.display());
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "record") {
        if let Err(e) = headless_record(&args[2..]) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(target_os = "linux")]
    webkit_workarounds();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(true) => {
                let state = window.state::<AppState>();
                let mut recording = state.recording.lock().unwrap();
                if let Some(r) = recording.as_mut() {
                    if r.tail_from.is_none() && r.elapsed_ms() > 1000.0 {
                        r.tail_from = Some(Instant::now());
                    }
                }
            }
            WindowEvent::Destroyed => {
                if let Some(r) = window.state::<AppState>().recording.lock().unwrap().take() {
                    r.cancel();
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            get_capabilities,
            download_ffmpeg,
            list_sources,
            start_recording,
            stop_recording,
            cancel_recording,
            list_recent,
            open_project,
            save_project,
            delete_project,
            auto_zoom,
            compute_motion,
            export_video,
            cancel_export,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cinesnap");
}
