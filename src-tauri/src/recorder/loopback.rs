//! Audio that ffmpeg cannot capture: what the computer plays on Windows and
//! macOS (WASAPI loopback and Core Audio taps, through cpal), and the sound
//! of a single app (its PipeWire stream on Linux, process loopback on
//! Windows). Chunks go through a [`Tape`] so the file keeps its silences.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};

use super::tape::Tape;

/// Captured samples, interleaved, with the instant the first was captured.
struct Chunk {
    at: Instant,
    rate: u32,
    channels: u16,
    samples: Vec<f32>,
}

/// The capture instant of a buffer of `frames` that has just arrived.
fn arrived(frames: usize, rate: u32) -> Instant {
    Instant::now() - Duration::from_secs_f64(frames as f64 / rate.max(1) as f64)
}

pub struct Loopback {
    stop: Arc<AtomicBool>,
    capture: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<Result<()>>>,
    first: Arc<OnceLock<Instant>>,
    error: Arc<OnceLock<String>>,
}

impl Loopback {
    /// Start `capture` on its own thread; it sends chunks until `stop` is set.
    fn spawn(
        out: &Path,
        capture: impl FnOnce(mpsc::Sender<Chunk>, Arc<AtomicBool>) -> Result<()> + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let first = Arc::new(OnceLock::new());
        let error = Arc::new(OnceLock::new());
        let (tx, rx) = mpsc::channel::<Chunk>();
        let writer = {
            let (out, first, stop) = (out.to_owned(), first.clone(), stop.clone());
            std::thread::spawn(move || write(&out, rx, &first, &stop))
        };
        let capture = {
            let (stop, error) = (stop.clone(), error.clone());
            std::thread::spawn(move || {
                if let Err(e) = capture(tx, stop) {
                    let _ = error.set(format!("{e:#}"));
                }
            })
        };
        Self {
            stop,
            capture: Some(capture),
            writer: Some(writer),
            first,
            error,
        }
    }

    pub fn first_sample(&self) -> Option<Instant> {
        self.first.get().copied()
    }

    /// Block until audio arrives or the capture fails. A source that is
    /// merely quiet is not an error: loopback delivers nothing until
    /// something plays.
    pub fn check_started(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline && self.first_sample().is_none() {
            if let Some(e) = self.error.get() {
                return Err(anyhow!("{e}"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Stop capturing and finish the file, padded to now.
    pub fn stop(mut self) -> Result<()> {
        self.shutdown()
    }

    fn shutdown(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.capture.take() {
            let _ = t.join();
        }
        match self.writer.take().map(JoinHandle::join) {
            Some(Ok(r)) => r,
            Some(Err(_)) => Err(anyhow!("the audio writer crashed")),
            None => Ok(()),
        }
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

/// Write chunks as they come. The file is only created once the format is
/// known, which is with the first chunk.
fn write(
    out: &Path,
    rx: mpsc::Receiver<Chunk>,
    first: &OnceLock<Instant>,
    stop: &AtomicBool,
) -> Result<()> {
    let mut tape: Option<Tape> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                let tape = match &mut tape {
                    Some(t) => t,
                    None => tape.insert(Tape::create(out, chunk.channels, chunk.rate)?),
                };
                tape.push(chunk.at, &chunk.samples)?;
                if let Some(t) = tape.first_sample() {
                    let _ = first.set(t);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if !stop.load(Ordering::SeqCst) => {}
            Err(_) => break,
        }
    }
    match tape {
        Some(t) => t.finish(Instant::now()),
        None => Ok(()),
    }
}

// ---- everything the computer plays: cpal --------------------------------------

/// What the default output device plays (Windows, and macOS 14.6 or newer).
#[cfg(not(target_os = "linux"))]
pub fn system(out: &Path) -> Result<Loopback> {
    let lb = Loopback::spawn(out, |tx, stop| {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host()
            .default_output_device()
            .context("there is no audio output device")?;
        let config = device
            .default_output_config()
            .context("reading the output format")?;
        let (rate, channels) = (config.sample_rate(), config.channels());
        let format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let on_error = |e| eprintln!("system audio: {e}");
        let send = move |samples: Vec<f32>| {
            let frames = samples.len() / channels.max(1) as usize;
            let _ = tx.send(Chunk {
                at: arrived(frames, rate),
                rate,
                channels,
                samples,
            });
        };
        // On an output device, cpal records what it plays.
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                stream_config,
                move |d: &[f32], _: &_| send(d.to_vec()),
                on_error,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                stream_config,
                move |d: &[i16], _: &_| send(d.iter().map(|&s| s as f32 / 32768.0).collect()),
                on_error,
                None,
            ),
            cpal::SampleFormat::I32 => device.build_input_stream(
                stream_config,
                move |d: &[i32], _: &_| {
                    send(d.iter().map(|&s| s as f32 / 2_147_483_648.0).collect())
                },
                on_error,
                None,
            ),
            other => return Err(anyhow!("unsupported output format {other}")),
        }
        .context("system audio capture is not available")?;
        stream.play().context("starting system audio capture")?;
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    });
    lb.check_started(Duration::from_millis(400))?;
    Ok(lb)
}

// ---- one app: PipeWire --------------------------------------------------------

/// What one app plays: its PipeWire output stream, by object serial.
#[cfg(target_os = "linux")]
pub fn app(serial: &str, out: &Path) -> Result<Loopback> {
    use pipewire as pw;
    use pw::spa;

    let serial = serial.to_owned();
    let lb = Loopback::spawn(out, move |tx, stop| {
        pw::init();
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let stream = pw::stream::StreamBox::new(
            &core,
            "cinesnap-app-audio",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Audio",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Production",
                "target.object" => serial,
                // Never fall back to the microphone if the app goes away.
                "node.dont-reconnect" => "true",
            },
        )?;
        let _listener = stream
            .add_local_listener_with_user_data(spa::param::audio::AudioInfoRaw::new())
            .param_changed(|_, format, id, param| {
                let Some(param) = param else { return };
                if id == spa::param::ParamType::Format.as_raw() {
                    let _ = format.parse(param);
                }
            })
            .process(move |stream, format| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let (rate, channels) = (format.rate(), format.channels() as u16);
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let size = data.chunk().size() as usize;
                let Some(bytes) = data.data() else { return };
                if rate == 0 || channels == 0 || size == 0 {
                    return;
                }
                let samples: Vec<f32> = bytes[..size.min(bytes.len())]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|b| f32::from_le_bytes(*b))
                    .collect();
                let frames = samples.len() / channels as usize;
                let _ = tx.send(Chunk {
                    at: arrived(frames, rate),
                    rate,
                    channels,
                    samples,
                });
            })
            .register()?;

        let mut info = spa::param::audio::AudioInfoRaw::new();
        info.set_format(spa::param::audio::AudioFormat::F32LE);
        let format = spa::pod::serialize::PodSerializer::serialize(
            std::io::Cursor::new(Vec::new()),
            &spa::pod::Value::Object(spa::pod::Object {
                type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                id: spa::param::ParamType::EnumFormat.as_raw(),
                properties: info.into(),
            }),
        )
        .map_err(|_| anyhow!("building the audio format"))?
        .0
        .into_inner();
        let mut params = [spa::pod::Pod::from_bytes(&format).context("audio format")?];
        stream.connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        let timer = mainloop.loop_().add_timer({
            let mainloop = mainloop.clone();
            move |_| {
                if stop.load(Ordering::SeqCst) {
                    mainloop.quit();
                }
            }
        });
        timer
            .update_timer(
                Some(Duration::from_millis(20)),
                Some(Duration::from_millis(20)),
            )
            .into_result()?;
        mainloop.run();
        Ok(())
    });
    lb.check_started(Duration::from_millis(300))?;
    Ok(lb)
}

// ---- one app: process loopback (Windows 10 2004 and newer) ----------------------

#[cfg(target_os = "windows")]
pub fn app(pid: &str, out: &Path) -> Result<Loopback> {
    let pid: u32 = pid.parse().context("not a process id")?;
    let lb = Loopback::spawn(out, move |tx, stop| windows_app::capture(pid, tx, &stop));
    lb.check_started(Duration::from_millis(500))?;
    Ok(lb)
}

#[cfg(target_os = "macos")]
pub fn app(_pid: &str, _out: &Path) -> Result<Loopback> {
    Err(anyhow!(
        "recording a single app's sound is not supported on macOS yet"
    ))
}

#[cfg(target_os = "windows")]
mod windows_app {
    use super::*;
    use windows::core::{implement, Interface};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, BLOB, COINIT_MULTITHREADED};
    use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
    use windows::Win32::System::Variant::VT_BLOB;

    const RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;

    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Done(HANDLE);

    impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
        fn ActivateCompleted(
            &self,
            _: windows::core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            unsafe { SetEvent(self.0) }
        }
    }

    /// An audio client that hears `pid` and the processes it started.
    unsafe fn activate(pid: u32) -> Result<IAudioClient> {
        let params = AUDIOCLIENT_ACTIVATION_PARAMS {
            ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
            Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                    TargetProcessId: pid,
                    ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                },
            },
        };
        let mut prop = PROPVARIANT::default();
        {
            let inner = &mut *prop.Anonymous.Anonymous;
            inner.vt = VT_BLOB;
            inner.Anonymous.blob = BLOB {
                cbSize: std::mem::size_of_val(&params) as u32,
                pBlobData: &params as *const _ as *mut u8,
            };
        }
        let event = CreateEventW(None, false, false, None)?;
        let handler: IActivateAudioInterfaceCompletionHandler = Done(event).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop),
            &handler,
        )?;
        let waited = WaitForSingleObject(event, 5000);
        let _ = CloseHandle(event);
        anyhow::ensure!(
            waited == WAIT_OBJECT_0,
            "the app's audio did not open in time"
        );
        let mut hr = windows::core::HRESULT(0);
        let mut unknown = None;
        op.GetActivateResult(&mut hr, &mut unknown)?;
        hr.ok().context("that app's audio cannot be recorded")?;
        Ok(unknown.context("no audio client")?.cast()?)
    }

    pub fn capture(pid: u32, tx: mpsc::Sender<Chunk>, stop: &AtomicBool) -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let result = run(pid, tx, stop);
            CoUninitialize();
            result
        }
    }

    unsafe fn run(pid: u32, tx: mpsc::Sender<Chunk>, stop: &AtomicBool) -> Result<()> {
        let client = activate(pid)?;
        // A process loopback client has no mix format of its own; it
        // converts to whatever is asked for.
        let block = CHANNELS * 4;
        let format = WAVEFORMATEX {
            wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
            nChannels: CHANNELS,
            nSamplesPerSec: RATE,
            nAvgBytesPerSec: RATE * block as u32,
            nBlockAlign: block,
            wBitsPerSample: 32,
            cbSize: 0,
        };
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
            200_000, // 20 ms, in 100 ns units
            0,
            &format,
            None,
        )?;
        let event = CreateEventW(None, false, false, None)?;
        client.SetEventHandle(event)?;
        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        while !stop.load(Ordering::SeqCst) {
            if WaitForSingleObject(event, 100) != WAIT_OBJECT_0 {
                continue;
            }
            while capture.GetNextPacketSize()? > 0 {
                let mut data = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let n = frames as usize * CHANNELS as usize;
                let samples = if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || data.is_null()
                {
                    vec![0.0; n]
                } else {
                    std::slice::from_raw_parts(data as *const f32, n).to_vec()
                };
                capture.ReleaseBuffer(frames)?;
                let _ = tx.send(Chunk {
                    at: arrived(frames as usize, RATE),
                    rate: RATE,
                    channels: CHANNELS,
                    samples,
                });
            }
        }
        let _ = client.Stop();
        let _ = CloseHandle(event);
        Ok(())
    }
}
