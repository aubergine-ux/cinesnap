#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod devices;
#[cfg(target_os = "windows")]
mod dxgi;
mod hotkeys;
mod media;
mod recorder;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cinesnap_core::clips::{self, Imported};
use cinesnap_core::events::EventLog;
use cinesnap_core::export::{export, Cancelled, ExportOptions};
use cinesnap_core::ffmpeg::{self, Encoder};
use cinesnap_core::motion::{self, Kept, MotionTracks};
use cinesnap_core::project::{Project, Transcript, Zoom, PROJECT_FILE};
use cinesnap_core::share::{self, Shared};
use cinesnap_core::transcript::{self, Suggestion};
use cinesnap_core::{activity, audio, music, presets, transcribe};
use recorder::{RecordOptions, Session, Target};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

#[derive(Default)]
struct AppState {
    recording: Mutex<Option<Session>>,
    /// When the pointer went onto the control bar, if it is still there:
    /// stopping from the bar cuts the reach for it.
    bar_hover: Mutex<Option<Instant>>,
    /// Guards against two overlapping `start_recording` calls.
    starting: AtomicBool,
    encoder: Mutex<Option<Encoder>>,
    export_cancel: Arc<AtomicBool>,
    /// The event logs of the project being edited, with the clips they
    /// belong to (logs never change on disk).
    log: Mutex<Option<LogCache>>,
    /// Cancel flags of running background jobs, by kind.
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    media_base: Option<String>,
}

/// Bundle path, clip ids, and the logs of those clips.
type LogCache = (PathBuf, Vec<String>, Arc<Vec<EventLog>>);

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

fn event_logs(state: &AppState, dir: &Path, project: &Project) -> Arc<Vec<EventLog>> {
    let ids: Vec<String> = project.clips.iter().map(|c| c.id.clone()).collect();
    let mut cache = state.log.lock().unwrap();
    if let Some((path, cached, logs)) = cache.as_ref() {
        if path == dir && *cached == ids {
            return logs.clone();
        }
    }
    let logs = Arc::new(motion::load_logs(dir, project));
    *cache = Some((dir.to_owned(), ids, logs.clone()));
    logs
}

/// Start a background job of the given kind, cancelling one of the same
/// kind that is still running. Returns its cancel flag.
fn start_job(state: &AppState, kind: &str) -> Arc<AtomicBool> {
    let flag = Arc::new(AtomicBool::new(false));
    if let Some(old) = state
        .jobs
        .lock()
        .unwrap()
        .insert(kind.to_owned(), flag.clone())
    {
        old.store(true, Ordering::SeqCst);
    }
    flag
}

/// Progress reporter for a job: emits `job://progress` at most ~15 times a
/// second, and always for the final value.
fn job_progress<'a>(app: &'a AppHandle, kind: &'a str, stage: &'a str) -> impl FnMut(f64) + 'a {
    let mut last = Instant::now() - Duration::from_secs(1);
    move |fraction: f64| {
        if fraction >= 1.0 || last.elapsed() > Duration::from_millis(66) {
            last = Instant::now();
            let _ = app.emit(
                "job://progress",
                json!({ "job": kind, "stage": stage, "fraction": fraction }),
            );
        }
    }
}

fn job_error(e: anyhow::Error) -> String {
    if e.is::<Cancelled>() {
        "cancelled".to_owned()
    } else {
        err(e)
    }
}

fn read_json(app: &AppHandle, file: &str) -> serde_json::Value {
    data_dir(app)
        .and_then(|d| std::fs::read_to_string(d.join(file)).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn write_json(app: &AppHandle, file: &str, value: &serde_json::Value) -> CmdResult<()> {
    let dir = data_dir(app).ok_or("no data directory")?;
    std::fs::create_dir_all(&dir).map_err(err)?;
    let tmp = dir.join(format!("{file}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec_pretty(value).map_err(err)?).map_err(err)?;
    std::fs::rename(&tmp, dir.join(file)).map_err(err)
}

#[derive(Serialize)]
struct FfmpegInfo {
    path: String,
    version: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    platform: &'static str,
    cursor: &'static str,
    clicks: &'static str,
    ffmpeg: Option<FfmpegInfo>,
    encoder: String,
    notes: Vec<String>,
    /// Base URL of the loopback media server, where one is needed.
    media_base: Option<String>,
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
        media_base: state.media_base.clone(),
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

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RecordingStatus {
    elapsed_ms: f64,
    paused: bool,
    countdown_ms: f64,
    segments: usize,
}

fn status(s: &Session) -> RecordingStatus {
    RecordingStatus {
        elapsed_ms: s.elapsed_ms(),
        paused: s.is_paused(),
        countdown_ms: s.countdown_ms(),
        segments: s.segments(),
    }
}

/// Bindings from the settings, applied to every window's hotkeys.
fn install_hotkeys(app: &AppHandle) -> Vec<String> {
    let bindings = hotkeys::Bindings::from_settings(&get_settings(app.clone()));
    let handle = app.clone();
    hotkeys::install(
        app,
        &bindings,
        Arc::new(move |action| on_hotkey(&handle, action)),
    )
}

fn on_hotkey(app: &AppHandle, action: hotkeys::Action) {
    use hotkeys::Action;
    let state = app.state::<AppState>();
    let recording = state.recording.lock().unwrap().is_some();
    match action {
        Action::Toggle if recording => {
            let app = app.clone();
            std::thread::spawn(move || {
                let _ = finish_recording(&app, false);
            });
        }
        // Starting needs the options chosen on the home screen.
        Action::Toggle => {
            let _ = app.emit_to("main", "hotkey://start", ());
        }
        Action::Pause if recording => {
            let app = app.clone();
            std::thread::spawn(move || {
                let paused = with_session(&app, |s| Ok(s.is_paused())).unwrap_or(false);
                let result = if paused {
                    with_session(&app, Session::resume)
                } else {
                    with_session(&app, Session::pause)
                };
                if let Err(e) = result {
                    let _ = app.emit("recording://error", e);
                }
            });
        }
        Action::Marker if recording => {
            let _ = with_session(app, |s| {
                s.marker();
                Ok(())
            });
            let _ = app.emit("recording://marker", ());
        }
        _ => {}
    }
}

fn with_session<T>(
    app: &AppHandle,
    f: impl FnOnce(&mut Session) -> anyhow::Result<T>,
) -> CmdResult<T> {
    let state = app.state::<AppState>();
    let mut guard = state.recording.lock().unwrap();
    let session = guard.as_mut().ok_or("nothing is being recorded")?;
    f(session).map_err(err)
}

const BAR: &str = "bar";

/// The floating control bar. Windows and macOS keep it out of the recording;
/// elsewhere it is put on a screen that is not being recorded when there is
/// one.
fn open_bar(app: &AppHandle, avoid: &[(i32, i32, u32, u32)]) {
    use tauri::{PhysicalPosition, WebviewUrl, WebviewWindowBuilder};
    let Ok(bar) = WebviewWindowBuilder::new(app, BAR, WebviewUrl::App("index.html#bar".into()))
        .title("Cinesnap")
        .inner_size(392.0, 52.0)
        .resizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .content_protected(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .build()
    else {
        return;
    };
    let screens = devices::screens();
    let free = screens.iter().find(|s| {
        !avoid.iter().any(|&(x, y, w, h)| {
            s.x < x + w as i32
                && x < s.x + s.width as i32
                && s.y < y + h as i32
                && y < s.y + s.height as i32
        })
    });
    let excluded = !cfg!(target_os = "linux");
    let screen = if excluded { None } else { free }
        .or_else(|| {
            screens
                .iter()
                .find(|s| avoid.first().is_some_and(|a| (s.x, s.y) == (a.0, a.1)))
        })
        .or(screens.first());
    if let (Some(s), Ok(size)) = (screen, bar.outer_size()) {
        let x = s.x + (s.width as i32 - size.width as i32) / 2;
        let _ = bar.set_position(PhysicalPosition::new(x, s.y + 16));
    }
}

fn close_bar(app: &AppHandle) {
    if let Some(bar) = app.get_webview_window(BAR) {
        let _ = bar.destroy();
    }
}

/// Whether the bar would show up in the recording (no way to exclude it,
/// and no other screen to put it on).
fn bar_in_picture(avoid: &[(i32, i32, u32, u32)]) -> bool {
    cfg!(target_os = "linux") && (devices::is_wayland() || devices::screens().len() <= avoid.len())
}

#[tauri::command(async)]
fn start_recording(
    app: AppHandle,
    state: State<AppState>,
    mut options: RecordOptions,
) -> CmdResult<()> {
    if state.recording.lock().unwrap().is_some() || state.starting.swap(true, Ordering::SeqCst) {
        return Err("a recording is already in progress".into());
    }
    #[cfg(target_os = "linux")]
    if devices::is_wayland() {
        install_hotkeys(&app);
    }
    let settings = get_settings(app.clone());
    options.hidden_combos = hotkeys::Bindings::from_settings(&settings).labels();
    let keep_bar = settings["keepBarVisible"].as_bool().unwrap_or(false);
    let result: CmdResult<Vec<(i32, i32, u32, u32)>> = (|| {
        let ffmpeg = find_ffmpeg(&app)?;
        let enc = encoder(&state, &ffmpeg);
        let name = chrono::Local::now()
            .format("Recording %Y-%m-%d %H.%M.%S.cinesnap")
            .to_string();
        let dir = recordings_dir(&app)?.join(name);
        let session = Session::start(&ffmpeg, enc, &dir, options).map_err(err)?;
        let screens = session.screens();
        *state.recording.lock().unwrap() = Some(session);
        Ok(screens)
    })();
    state.starting.store(false, Ordering::SeqCst);
    let screens = result?;

    // Get out of the picture; the bar and the hotkeys take over.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.minimize();
    }
    open_bar(&app, &screens);
    let hide_bar_after = (bar_in_picture(&screens) && !keep_bar).then_some(());
    std::thread::spawn(move || loop {
        let current = app
            .state::<AppState>()
            .recording
            .lock()
            .unwrap()
            .as_ref()
            .map(status);
        let Some(st) = current else { break };
        // A bar that would be filmed only stays for the countdown.
        if hide_bar_after.is_some() && st.elapsed_ms > st.countdown_ms + 300.0 {
            if let Some(bar) = app.get_webview_window(BAR) {
                if bar.is_visible().unwrap_or(false) {
                    let _ = bar.hide();
                }
            }
        }
        let _ = app.emit("recording://tick", st);
        std::thread::sleep(Duration::from_millis(200));
    });
    Ok(())
}

/// Stop, assemble the project and tell every window. `from_bar` means the
/// stop button on the bar, so the reach for it is cut.
fn finish_recording(app: &AppHandle, from_bar: bool) -> CmdResult<String> {
    let state = app.state::<AppState>();
    let hover = state.bar_hover.lock().unwrap().take();
    let mut session = state
        .recording
        .lock()
        .unwrap()
        .take()
        .ok_or("nothing is being recorded")?;
    if from_bar {
        session.tail_from = hover.or(session.tail_from);
    }
    close_bar(app);
    let result = session.stop().map_err(err);
    match &result {
        Ok(dir) => {
            let _ = app.emit("recording://stopped", dir.display().to_string());
        }
        Err(e) => {
            let _ = app.emit("recording://error", e.clone());
        }
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    result.map(|d| d.display().to_string())
}

#[tauri::command(async)]
fn stop_recording(app: AppHandle, from_bar: Option<bool>) -> CmdResult<String> {
    finish_recording(&app, from_bar.unwrap_or(false))
}

#[tauri::command(async)]
fn cancel_recording(app: AppHandle, state: State<AppState>) {
    if let Some(session) = state.recording.lock().unwrap().take() {
        session.cancel();
    }
    close_bar(&app);
    let _ = app.emit("recording://cancelled", ());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
    }
}

#[tauri::command(async)]
fn pause_recording(app: AppHandle) -> CmdResult<RecordingStatus> {
    with_session(&app, |s| {
        s.pause()?;
        Ok(status(s))
    })
}

#[tauri::command(async)]
fn resume_recording(app: AppHandle) -> CmdResult<RecordingStatus> {
    with_session(&app, |s| {
        s.resume()?;
        Ok(status(s))
    })
}

#[tauri::command]
fn add_marker(app: AppHandle) -> CmdResult<()> {
    with_session(&app, |s| {
        s.marker();
        Ok(())
    })
}

/// Record another screen or window from now on.
#[tauri::command(async)]
fn switch_target(app: AppHandle, target: Target) -> CmdResult<()> {
    with_session(&app, |s| s.switch(target))
}

#[tauri::command]
fn bar_hover(state: State<AppState>, inside: bool) {
    *state.bar_hover.lock().unwrap() = inside.then(Instant::now);
}

#[tauri::command(async)]
fn recording_status(state: State<AppState>) -> Option<RecordingStatus> {
    state.recording.lock().unwrap().as_ref().map(status)
}

#[tauri::command(async)]
fn apply_hotkeys(app: AppHandle) -> Vec<String> {
    install_hotkeys(&app)
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
                duration_ms: project.duration_ms(),
                created_at: project.created_at,
            })
        })
        .collect();
    list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    list
}

/// A recording passed on the command line (`cinesnap demo.cinesnap`).
#[tauri::command]
fn initial_project() -> Option<String> {
    let arg = std::env::args().nth(1)?;
    let dir = std::fs::canonicalize(arg).ok()?;
    dir.join(PROJECT_FILE)
        .is_file()
        .then(|| dir.display().to_string())
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
    let dir = Path::new(&path);
    motion::auto_zoom(
        &project,
        &event_logs(&state, dir, &project),
        &activity::load(dir, &project),
    )
}

#[tauri::command(async)]
fn compute_motion(state: State<AppState>, path: String, mut project: Project) -> MotionTracks {
    project.sanitize();
    let dir = Path::new(&path);
    motion::compute_motion(
        &project,
        &event_logs(&state, dir, &project),
        &activity::load(dir, &project),
    )
}

/// Find out where things change on screen in recordings not analysed yet.
/// Returns whether there was anything to do.
#[tauri::command(async)]
fn analyze_project(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    project: Project,
) -> CmdResult<bool> {
    let dir = Path::new(&path);
    if !activity::missing(dir, &project) {
        return Ok(false);
    }
    let ffmpeg = find_ffmpeg(&app)?;
    let cancel = start_job(&state, "activity");
    activity::ensure(
        dir,
        &project,
        &ffmpeg,
        &cancel,
        &mut job_progress(&app, "activity", "analyse"),
    )
    .map_err(job_error)?;
    Ok(true)
}

/// Save for the last time and drop the files of clips that were removed.
#[tauri::command(async)]
fn close_project(path: String, mut project: Project) -> CmdResult<()> {
    project.sanitize();
    let dir = Path::new(&path);
    project.save(dir).map_err(err)?;
    clips::remove_orphans(dir, &project);
    Ok(())
}

/// Copy another recording into this project as a new clip.
#[tauri::command(async)]
fn import_clip(path: String, project: Project, from: String) -> CmdResult<Imported> {
    let (dir, from) = (Path::new(&path), Path::new(&from));
    if std::fs::canonicalize(dir).ok() == std::fs::canonicalize(from).ok() {
        return Err("that is the recording you are editing".into());
    }
    clips::import_recording(dir, &project, from).map_err(err)
}

#[tauri::command]
fn export_presets() -> &'static [presets::ExportPreset] {
    &presets::PRESETS
}

#[tauri::command]
fn music_tracks() -> &'static [music::TrackInfo] {
    music::tracks()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpeechModel {
    #[serde(flatten)]
    info: transcribe::ModelInfo,
    downloaded: bool,
}

#[derive(Serialize)]
struct Speech {
    available: bool,
    models: Vec<SpeechModel>,
}

fn models_dir(app: &AppHandle) -> CmdResult<PathBuf> {
    Ok(data_dir(app).ok_or("no data directory")?.join("models"))
}

#[tauri::command(async)]
fn speech_models(app: AppHandle) -> Speech {
    let dir = models_dir(&app).ok();
    Speech {
        available: transcribe::available(),
        models: transcribe::MODELS
            .iter()
            .map(|m| SpeechModel {
                info: *m,
                downloaded: dir
                    .as_ref()
                    .is_some_and(|d| transcribe::model_path(d, m.id).is_file()),
            })
            .collect(),
    }
}

/// Transcribe the project's speech on this machine. Downloads the model
/// first if it is not there yet (the only step that uses the network).
#[tauri::command(async)]
fn transcribe_project(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    project: Project,
    model: String,
    language: Option<String>,
) -> CmdResult<Transcript> {
    let ffmpeg = find_ffmpeg(&app)?;
    let cancel = start_job(&state, "transcribe");
    let models = models_dir(&app)?;
    let file = transcribe::download_model(
        &models,
        &model,
        &mut job_progress(&app, "transcribe", "download"),
        &cancel,
    )
    .map_err(job_error)?;
    transcribe::transcribe(
        Path::new(&path),
        &project,
        &ffmpeg,
        &file,
        language.as_deref().filter(|l| !l.is_empty()),
        &mut job_progress(&app, "transcribe", "listen"),
        &cancel,
    )
    .map_err(job_error)
}

/// Build (or find) the soundtrack for the editor preview. Returns its path,
/// or nothing when the project is silent.
#[tauri::command(async)]
fn audio_preview(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    mut project: Project,
) -> CmdResult<Option<String>> {
    project.sanitize();
    let ffmpeg = find_ffmpeg(&app)?;
    let cancel = start_job(&state, "audio");
    audio::preview_mix(
        Path::new(&path),
        &project,
        &ffmpeg,
        &mut job_progress(&app, "audio", "mix"),
        &cancel,
    )
    .map(|p| p.map(|p| p.display().to_string()))
    .map_err(job_error)
}

/// Filler words and long silences that could be cut, for review.
#[tauri::command(async)]
fn suggest_cuts(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    mut project: Project,
    fillers: bool,
    silences: bool,
    min_silence_ms: f64,
) -> CmdResult<Vec<Suggestion>> {
    project.sanitize();
    let kept = Kept::new(project.duration_ms(), &project.cuts);
    let mut out = Vec::new();
    if fillers {
        if let Some(t) = &project.transcript {
            out.extend(transcript::find_fillers(&t.words, &kept));
        }
    }
    if silences {
        let ffmpeg = find_ffmpeg(&app)?;
        let cancel = start_job(&state, "silence");
        let min = min_silence_ms.clamp(300.0, 60_000.0);
        let found = audio::detect_silences(Path::new(&path), &project, &ffmpeg, min, &cancel)
            .map_err(job_error)?;
        out.extend(transcript::silence_cuts(&found, &kept, min, 200.0));
    }
    out.sort_by(|a, b| a.start_ms.total_cmp(&b.start_ms));
    Ok(out)
}

#[tauri::command(async)]
fn export_captions(mut project: Project, out_path: String) -> CmdResult<usize> {
    project.sanitize();
    let words = project
        .transcript
        .as_ref()
        .map(|t| t.words.as_slice())
        .ok_or("there is no transcript yet")?;
    let kept = Kept::new(project.duration_ms(), &project.cuts);
    let lines = transcript::caption_lines(words, &kept, project.captions.max_chars);
    let text = if out_path.to_ascii_lowercase().ends_with(".vtt") {
        transcript::vtt(&lines)
    } else {
        transcript::srt(&lines)
    };
    std::fs::write(&out_path, text).map_err(err)?;
    Ok(lines.len())
}

/// Saved looks, shared by every project on this machine.
#[tauri::command(async)]
fn list_styles(app: AppHandle) -> serde_json::Value {
    match read_json(&app, "styles.json") {
        serde_json::Value::Array(a) => serde_json::Value::Array(a),
        _ => json!([]),
    }
}

#[tauri::command(async)]
fn save_styles(app: AppHandle, styles: serde_json::Value) -> CmdResult<()> {
    if !styles.is_array() {
        return Err("styles must be a list".into());
    }
    write_json(&app, "styles.json", &styles)
}

#[tauri::command(async)]
fn get_settings(app: AppHandle) -> serde_json::Value {
    match read_json(&app, "settings.json") {
        serde_json::Value::Object(o) => serde_json::Value::Object(o),
        _ => json!({}),
    }
}

#[tauri::command(async)]
fn save_settings(app: AppHandle, settings: serde_json::Value) -> CmdResult<()> {
    if !settings.is_object() {
        return Err("settings must be an object".into());
    }
    write_json(&app, "settings.json", &settings)
}

/// Upload an exported file to the share server from the settings.
#[tauri::command(async)]
fn share_video(
    app: AppHandle,
    state: State<AppState>,
    file: String,
    title: String,
) -> CmdResult<Shared> {
    let settings = get_settings(app.clone());
    let field = |k: &str| settings[k].as_str().unwrap_or_default().trim().to_owned();
    let (server, token) = (field("shareServer"), field("shareToken"));
    if server.is_empty() || token.is_empty() {
        return Err("set the share server address and token first".into());
    }
    let cancel = start_job(&state, "share");
    share::upload(
        &server,
        &token,
        Path::new(&file),
        &title,
        &mut job_progress(&app, "share", "upload"),
        &cancel,
    )
    .map_err(job_error)
}

#[tauri::command]
fn cancel_job(state: State<AppState>, job: String) {
    if let Some(flag) = state.jobs.lock().unwrap().get(&job) {
        flag.store(true, Ordering::SeqCst);
    }
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

/// Diagnostics from the frontend (media errors), shown when the app is
/// started from a terminal.
#[tauri::command]
fn client_log(message: String) {
    eprintln!("ui: {message}");
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
    if nvidia
        && devices::is_wayland()
        && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

const RECORD_USAGE: &str = "usage: cinesnap record <dir.cinesnap> [--seconds N] [--fps N] [--cfr] \
    [--all-screens] [--mic ID] [--system ID] [--webcam ID] [--pause-at N]";

/// `cinesnap record <dir> …`: record without opening the window. Handy for
/// scripts and for checking that capture works on a machine.
fn headless_record(args: &[String]) -> anyhow::Result<()> {
    use anyhow::Context;
    let mut dir = None;
    let (mut seconds, mut fps, mut mic, mut webcam) = (5.0f64, 60u32, None, None);
    let (mut system_audio, mut vfr, mut target, mut pause_at) = (None, true, "screen", None);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--seconds" => seconds = value()?.parse()?,
            "--fps" => fps = value()?.parse()?,
            "--mic" => mic = Some(value()?.clone()),
            "--system" => system_audio = Some(value()?.clone()),
            "--webcam" => webcam = Some(value()?.clone()),
            "--pause-at" => pause_at = Some(value()?.parse::<f64>()?),
            "--cfr" => vfr = false,
            "--all-screens" => target = "all",
            other if other.starts_with("--") => {
                anyhow::bail!("unknown option {other}\n{RECORD_USAGE}")
            }
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let dir = dir.context(RECORD_USAGE)?;
    anyhow::ensure!(
        seconds.is_finite() && seconds > 0.0 && seconds <= 86_400.0,
        "--seconds must be a positive number"
    );
    let ffmpeg = ffmpeg::locate(None).context("ffmpeg not found")?;
    let (wants_mic, wants_webcam, wants_system) =
        (mic.is_some(), webcam.is_some(), system_audio.is_some());
    let options = RecordOptions {
        target: Target {
            kind: target.into(),
            id: None,
        },
        region: None,
        mic,
        webcam,
        system_audio,
        app_audio: None,
        app_name: None,
        fps,
        vfr,
        countdown: 0,
        shortcuts: true,
        hidden_combos: vec![],
    };
    let mut session = Session::start(&ffmpeg, Encoder::detect(&ffmpeg), &dir, options)?;
    eprintln!("recording for {seconds} s…");
    match pause_at {
        Some(at) if at < seconds => {
            std::thread::sleep(Duration::from_secs_f64(at));
            session.pause()?;
            eprintln!("paused for 1 s…");
            std::thread::sleep(Duration::from_secs(1));
            session.resume()?;
            std::thread::sleep(Duration::from_secs_f64(seconds - at));
        }
        _ => std::thread::sleep(Duration::from_secs_f64(seconds)),
    }
    let dir = session.stop()?;
    let source = Project::load(&dir)?.source;
    if wants_mic && source.mic.is_none() {
        eprintln!("warning: the microphone delivered no audio and was left out");
    }
    if wants_webcam && source.webcam.is_none() {
        eprintln!("warning: the webcam delivered no video and was left out");
    }
    if wants_system && source.system.is_none() {
        eprintln!("warning: no system sound was played, so there is no system track");
    }
    println!("{}", dir.display());
    Ok(())
}

/// `cinesnap doctor`: print what this machine and session support.
fn doctor() {
    let ffmpeg = ffmpeg::locate(None);
    let (cursor, clicks, notes) = devices::input_capabilities();
    println!("platform  {}", devices::platform());
    match &ffmpeg {
        Some(f) => {
            println!(
                "ffmpeg    {} ({})",
                f.display(),
                ffmpeg::version(f).unwrap_or_default()
            );
            println!("encoder   {}", Encoder::detect(f).name());
        }
        None => println!("ffmpeg    not found (the app can download it)"),
    }
    println!("cursor    {cursor}");
    println!("clicks    {clicks}");
    let sources = devices::sources(ffmpeg.as_deref());
    if sources.portal {
        println!("capture   screen or window, chosen in the system dialog");
    }
    for s in &sources.screens {
        println!(
            "screen    {} {}x{} at {},{}{}",
            s.name,
            s.width,
            s.height,
            s.x,
            s.y,
            if s.hdr { " (HDR)" } else { "" }
        );
    }
    println!("windows   {}", sources.windows.len());
    for d in &sources.mics {
        println!("mic       {}", d.name);
    }
    for d in &sources.webcams {
        println!("webcam    {}", d.name);
    }
    for d in &sources.outputs {
        println!("output    {} ({})", d.name, d.id);
    }
    for d in &sources.apps {
        println!("app       {} ({})", d.name, d.id);
    }
    for n in notes {
        println!("note      {n}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|a| a == "doctor") {
        return doctor();
    }
    let command = args.get(1).map(String::as_str).unwrap_or_default();
    let headless = match command {
        "record" => Some(headless_record(&args[2..])),
        // `cinesnap render …`, `cinesnap autozoom …`: the same command line
        // as the standalone cinesnap-cli.
        c if cinesnap_core::cli::COMMANDS.contains(&c) => {
            Some(cinesnap_core::cli::run(std::env::args_os()))
        }
        _ => None,
    };
    if let Some(result) = headless {
        if let Err(e) = result {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }

    #[cfg(target_os = "linux")]
    webkit_workarounds();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState {
            // GStreamer (behind WebKitGTK's <video>) cannot read asset:// URLs.
            media_base: if cfg!(target_os = "linux") {
                media::serve()
            } else {
                None
            },
            ..Default::default()
        })
        .setup(|app| {
            // The portal asks the desktop to confirm; that waits for the
            // first recording instead of greeting every start.
            if !devices::is_wayland() {
                for note in install_hotkeys(app.handle()) {
                    eprintln!("hotkeys: {note}");
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::Focused(true) if window.label() == "main" => {
                let state = window.state::<AppState>();
                let mut recording = state.recording.lock().unwrap();
                if let Some(r) = recording.as_mut() {
                    if r.tail_from.is_none() && r.elapsed_ms() > 1000.0 {
                        r.tail_from = Some(Instant::now());
                    }
                }
            }
            WindowEvent::Destroyed if window.label() == "main" => {
                if let Some(r) = window.state::<AppState>().recording.lock().unwrap().take() {
                    r.cancel();
                }
                close_bar(window.app_handle());
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
            pause_recording,
            resume_recording,
            add_marker,
            switch_target,
            bar_hover,
            recording_status,
            apply_hotkeys,
            list_recent,
            initial_project,
            open_project,
            save_project,
            delete_project,
            auto_zoom,
            compute_motion,
            analyze_project,
            close_project,
            import_clip,
            export_presets,
            music_tracks,
            speech_models,
            transcribe_project,
            audio_preview,
            suggest_cuts,
            export_captions,
            list_styles,
            save_styles,
            get_settings,
            save_settings,
            share_video,
            cancel_job,
            export_video,
            cancel_export,
            client_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Cinesnap");
}
