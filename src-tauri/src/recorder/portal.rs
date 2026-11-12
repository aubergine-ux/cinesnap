//! Wayland capture: xdg-desktop-portal ScreenCast + PipeWire.
//!
//! The portal hands us a PipeWire node. Frames arrive whenever the compositor
//! has damage (so at an irregular rate) and a writer thread resamples them to
//! a constant frame rate into an ffmpeg encoder. When the compositor supports
//! cursor mode "metadata", the cursor is not in the frames; its position rides
//! along on each buffer and feeds the event log instead.

use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use ashpd::desktop::screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType};
use ashpd::desktop::PersistMode;
use cinesnap_core::events::EventKind;
use cinesnap_core::ffmpeg::{Encoder, FrameWriter, Quality};
use pipewire as pw;
use pw::spa;

use super::input::Sink;

/// Room for cursor bitmaps up to 256×256 in the buffer metadata.
const CURSOR_META_SIZE: i32 = (std::mem::size_of::<spa::sys::spa_meta_cursor>()
    + std::mem::size_of::<spa::sys::spa_meta_bitmap>()) as i32
    + 256 * 256 * 4;

#[derive(Default)]
struct Latest {
    data: Vec<u8>,
    width: u32,
    height: u32,
    pix_fmt: &'static str,
    seq: u64,
}

struct Shared {
    latest: Mutex<Latest>,
    arrived: Condvar,
    t0: OnceLock<Instant>,
    cursor: Arc<Mutex<(f64, f64)>>,
    error: Mutex<Option<String>>,
}

pub struct PortalCapture {
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
    quit: pw::channel::Sender<()>,
    close_session: mpsc::Sender<()>,
    pw_thread: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<Result<()>>>,
    /// Whether the cursor position is delivered as metadata (and the cursor
    /// is therefore absent from the video).
    pub cursor_metadata: bool,
}

/// Which cursor modes the running portal offers.
pub fn cursor_metadata_available() -> bool {
    tauri::async_runtime::block_on(async {
        let proxy = Screencast::new().await.ok()?;
        proxy.available_cursor_modes().await.ok()
    })
    .is_some_and(|m| m.contains(CursorMode::METADATA))
}

struct Portal {
    node_id: u32,
    fd: OwnedFd,
    cursor_metadata: bool,
    close: mpsc::Sender<()>,
}

/// Show the system's screen/window picker and open the stream.
fn open_portal() -> Result<Portal> {
    let (close, closed) = mpsc::channel::<()>();
    let (node_id, fd, cursor_metadata, proxy, session) = tauri::async_runtime::block_on(async {
        let proxy = Screencast::new().await?;
        let modes = proxy
            .available_cursor_modes()
            .await
            .unwrap_or(CursorMode::EMBEDDED);
        let metadata = modes.contains(CursorMode::METADATA);
        let session = proxy.create_session(Default::default()).await?;
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_cursor_mode(if metadata {
                        CursorMode::METADATA
                    } else {
                        CursorMode::EMBEDDED
                    })
                    .set_sources(SourceType::MONITOR | SourceType::WINDOW)
                    .set_multiple(false)
                    .set_persist_mode(PersistMode::DoNot),
            )
            .await?;
        let response = proxy
            .start(&session, None, Default::default())
            .await?
            .response()?;
        let stream = response
            .streams()
            .first()
            .ok_or(ashpd::Error::NoResponse)?
            .to_owned();
        let fd = proxy
            .open_pipe_wire_remote(&session, Default::default())
            .await?;
        Ok::<_, ashpd::Error>((stream.pipe_wire_node_id(), fd, metadata, proxy, session))
    })
    .map_err(|e| match e {
        ashpd::Error::Response(_) => anyhow!("screen selection was cancelled"),
        e => anyhow!("screen capture portal failed: {e}"),
    })?;

    // The cast lives as long as the portal session; keep it until told to stop.
    std::thread::spawn(move || {
        let _ = closed.recv();
        tauri::async_runtime::block_on(async {
            let _ = session.close().await;
        });
        drop(proxy);
    });
    Ok(Portal {
        node_id,
        fd,
        cursor_metadata,
        close,
    })
}

fn pix_fmt(format: spa::param::video::VideoFormat) -> Option<&'static str> {
    use spa::param::video::VideoFormat as F;
    Some(match format {
        F::BGRx => "bgr0",
        F::BGRA => "bgra",
        F::RGBx => "rgb0",
        F::RGBA => "rgba",
        _ => return None,
    })
}

fn serialize(object: spa::pod::Object) -> Vec<u8> {
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .expect("serializing a pod into memory cannot fail")
    .0
    .into_inner()
}

fn format_param() -> Vec<u8> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    use spa::utils::{Fraction, Rectangle};
    serialize(spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
        spa::pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        spa::pod::property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA,
        ),
        spa::pod::property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 16384,
                height: 16384
            }
        ),
        spa::pod::property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 60, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: 240, denom: 1 }
        ),
    ))
}

/// Ask for cursor metadata on every buffer.
fn cursor_meta_param() -> Vec<u8> {
    use spa::pod::{Property, Value};
    serialize(spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
        id: spa::param::ParamType::Meta.as_raw(),
        properties: vec![
            Property::new(
                spa::sys::SPA_PARAM_META_type,
                Value::Id(spa::utils::Id(spa::sys::SPA_META_Cursor)),
            ),
            Property::new(spa::sys::SPA_PARAM_META_size, Value::Int(CURSOR_META_SIZE)),
        ],
    })
}

struct StreamState {
    format: spa::param::video::VideoInfoRaw,
    shared: Arc<Shared>,
    sink: Sink,
    last_cursor: (i32, i32),
}

/// Handle one PipeWire buffer: cursor metadata first, then pixels.
///
/// # Safety
/// `pwb` must be a buffer just dequeued from a stream connected with
/// `MAP_BUFFERS`, and must not have been queued back yet.
unsafe fn handle_buffer(pwb: *mut pw::sys::pw_buffer, st: &mut StreamState) {
    let buf = (*pwb).buffer;
    if buf.is_null() {
        return;
    }
    if !(*buf).metas.is_null() {
        for meta in std::slice::from_raw_parts((*buf).metas, (*buf).n_metas as usize) {
            if meta.type_ != spa::sys::SPA_META_Cursor
                || meta.data.is_null()
                || (meta.size as usize) < std::mem::size_of::<spa::sys::spa_meta_cursor>()
            {
                continue;
            }
            let cursor = &*(meta.data as *const spa::sys::spa_meta_cursor);
            // id 0 means "no cursor information in this buffer".
            if cursor.id != 0 {
                let pos = (cursor.position.x, cursor.position.y);
                if pos != st.last_cursor {
                    st.last_cursor = pos;
                    let (x, y) = (pos.0 as f64, pos.1 as f64);
                    *st.shared.cursor.lock().unwrap() = (x, y);
                    st.sink.push(EventKind::Move { x, y });
                }
            }
        }
    }

    if (*buf).n_datas == 0 || (*buf).datas.is_null() {
        return;
    }
    let data = &*(*buf).datas;
    if data.data.is_null() || data.chunk.is_null() {
        return;
    }
    let chunk = &*data.chunk;
    let Some(fmt) = pix_fmt(st.format.format()) else {
        return;
    };
    let (w, h) = (
        st.format.size().width as usize,
        st.format.size().height as usize,
    );
    // Cursor-only updates carry an empty (or flagged) chunk.
    if chunk.size == 0
        || chunk.flags & spa::sys::SPA_CHUNK_FLAG_CORRUPTED as i32 != 0
        || w == 0
        || h == 0
    {
        return;
    }
    let row = w * 4;
    let stride = if chunk.stride > 0 {
        chunk.stride as usize
    } else {
        row
    };
    let offset = chunk.offset as usize;
    if stride < row || offset + stride * (h - 1) + row > data.maxsize as usize {
        return;
    }
    let src =
        std::slice::from_raw_parts((data.data as *const u8).add(offset), stride * (h - 1) + row);

    let mut latest = st.shared.latest.lock().unwrap();
    latest.data.resize(row * h, 0);
    if stride == row {
        latest.data.copy_from_slice(&src[..row * h]);
    } else {
        for (dst, line) in latest.data.chunks_exact_mut(row).zip(src.chunks(stride)) {
            dst.copy_from_slice(&line[..row]);
        }
    }
    latest.width = w as u32;
    latest.height = h as u32;
    latest.pix_fmt = fmt;
    latest.seq += 1;
    drop(latest);
    st.shared.arrived.notify_all();
}

fn run_pipewire(
    node_id: u32,
    fd: OwnedFd,
    want_cursor: bool,
    shared: Arc<Shared>,
    sink: Sink,
    quit: pw::channel::Receiver<()>,
) -> Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(fd, None)?;
    let _quit = quit.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let stream = pw::stream::StreamBox::new(
        &core,
        "cinesnap",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )?;
    let state = StreamState {
        format: Default::default(),
        shared,
        sink,
        last_cursor: (i32::MIN, i32::MIN),
    };
    let _listener = stream
        .add_local_listener_with_user_data(state)
        .param_changed(move |stream, st, id, param| {
            let Some(param) = param else { return };
            if id != spa::param::ParamType::Format.as_raw() {
                return;
            }
            use spa::param::format::{MediaSubtype, MediaType};
            match spa::param::format_utils::parse_format(param) {
                Ok((MediaType::Video, MediaSubtype::Raw)) => {}
                _ => return,
            }
            if st.format.parse(param).is_err() {
                return;
            }
            if want_cursor {
                let meta = cursor_meta_param();
                if let Some(pod) = spa::pod::Pod::from_bytes(&meta) {
                    let _ = stream.update_params(&mut [pod]);
                }
            }
        })
        .process(|stream, st| unsafe {
            let pwb = stream.dequeue_raw_buffer();
            if pwb.is_null() {
                return;
            }
            handle_buffer(pwb, st);
            stream.queue_raw_buffer(pwb);
        })
        .register()?;

    let format = format_param();
    let mut params = [spa::pod::Pod::from_bytes(&format).context("building format parameters")?];
    stream.connect(
        spa::utils::Direction::Input,
        Some(node_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;
    mainloop.run();
    Ok(())
}

/// Resample incoming frames to a constant frame rate and feed the encoder.
fn run_writer(
    ffmpeg: PathBuf,
    encoder: Encoder,
    fps: u32,
    out: PathBuf,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
) -> Result<()> {
    // Wait for the first frame to learn the size and pixel format.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut frame = Vec::new();
    let (width, height, fmt, mut seq) = loop {
        let latest = shared.latest.lock().unwrap();
        let (latest, _) = shared
            .arrived
            .wait_timeout(latest, Duration::from_millis(100))
            .unwrap();
        if latest.seq > 0 {
            frame.extend_from_slice(&latest.data);
            break (latest.width, latest.height, latest.pix_fmt, latest.seq);
        }
        drop(latest);
        if stop.load(Ordering::Relaxed) {
            bail!("recording stopped before the first frame arrived");
        }
        if let Some(e) = shared.error.lock().unwrap().clone() {
            bail!("{e}");
        }
        if Instant::now() > deadline {
            bail!("the compositor did not send any frames");
        }
    };

    let mut filters = vec!["crop=trunc(iw/2)*2:trunc(ih/2)*2".to_string()];
    filters.extend(encoder.filter().map(String::from));
    let mut writer = FrameWriter::spawn_fmt(
        &ffmpeg,
        fmt,
        width,
        height,
        fps as f64,
        &encoder.global_args(),
        |c| {
            c.args(["-vf", &filters.join(",")]);
            c.args(encoder.codec_args(Quality::Master, true));
            c.args(["-g", &fps.to_string()]).arg(&out);
        },
    )?;

    let t0 = Instant::now();
    let _ = shared.t0.set(t0);
    let mut written = 0u64;
    while !stop.load(Ordering::Relaxed) {
        {
            let latest = shared.latest.lock().unwrap();
            // A resized window changes the stream size; the file cannot, so
            // keep showing the last frame of the original size.
            if latest.seq != seq && latest.data.len() == frame.len() {
                frame.copy_from_slice(&latest.data);
                seq = latest.seq;
            }
        }
        // Frame k is shown at t0 + k/fps. If encoding fell behind, repeat the
        // frame until the file has caught up with the clock, so that the
        // event log and the audio stay aligned with the picture.
        let due = (t0.elapsed().as_secs_f64() * fps as f64) as u64 + 1;
        while written < due {
            writer.write(&frame)?;
            written += 1;
        }
        let next = t0 + Duration::from_secs_f64(written as f64 / fps as f64);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    writer.finish()
}

impl PortalCapture {
    /// Opens the system picker (blocking until the user answers) and starts
    /// capturing to `out`.
    pub fn start(
        ffmpeg: &Path,
        encoder: Encoder,
        fps: u32,
        out: &Path,
        sink: Sink,
    ) -> Result<Self> {
        let portal = open_portal()?;
        let shared = Arc::new(Shared {
            latest: Mutex::default(),
            arrived: Condvar::new(),
            t0: OnceLock::new(),
            cursor: Arc::new(Mutex::new((0.0, 0.0))),
            error: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (quit, quit_rx) = pw::channel::channel();

        let pw_thread = {
            let (shared, want_cursor) = (shared.clone(), portal.cursor_metadata);
            std::thread::spawn(move || {
                if let Err(e) = run_pipewire(
                    portal.node_id,
                    portal.fd,
                    want_cursor,
                    shared.clone(),
                    sink,
                    quit_rx,
                ) {
                    *shared.error.lock().unwrap() = Some(format!("PipeWire: {e}"));
                }
            })
        };
        let writer = {
            let (ffmpeg, out, shared, stop) = (
                ffmpeg.to_owned(),
                out.to_owned(),
                shared.clone(),
                stop.clone(),
            );
            std::thread::spawn(move || run_writer(ffmpeg, encoder, fps, out, shared, stop))
        };
        Ok(Self {
            shared,
            stop,
            quit,
            close_session: portal.close,
            pw_thread: Some(pw_thread),
            writer: Some(writer),
            cursor_metadata: portal.cursor_metadata,
        })
    }

    /// Block until the first frame has been written; returns its instant.
    pub fn wait_first_frame(&mut self, timeout: Duration) -> Result<Instant> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(t) = self.shared.t0.get() {
                return Ok(*t);
            }
            if self.writer.as_ref().is_some_and(|w| w.is_finished()) {
                return match self.writer.take().unwrap().join() {
                    Ok(Err(e)) => Err(e),
                    _ => Err(anyhow!("capture stopped unexpectedly")),
                };
            }
            if Instant::now() > deadline {
                bail!("capture did not start in time");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Live cursor position in video pixels, shared with the button tracker.
    pub fn cursor(&self) -> Arc<Mutex<(f64, f64)>> {
        self.shared.cursor.clone()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        let result = match self.writer.take().map(|w| w.join()) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err(anyhow!("the encoder thread crashed")),
            None => Ok(()),
        };
        let _ = self.quit.send(());
        if let Some(t) = self.pw_thread.take() {
            let _ = t.join();
        }
        let _ = self.close_session.send(());
        result
    }

    pub fn stop(mut self) -> Result<()> {
        self.shutdown()
    }
}

impl Drop for PortalCapture {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}
