//! Sound: cleaning up the voice track, and mixing voice and music into the
//! soundtrack of the finished video.
//!
//! Each recording's microphone track is processed once by ffmpeg (alignment,
//! noise reduction, EQ, loudness) and cached as a mono WAV in the clip's own
//! time. The mixer then walks the output timeline, splicing the voice at the
//! cuts with short fades and laying looped music underneath, lowered while
//! someone is speaking.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};

use crate::export::Cancelled;
use crate::ffmpeg;
use crate::motion::Kept;
use crate::project::{AudioSettings, Music, Project, RecordingClip};

pub const RATE: u32 = 48_000;
const RATE_F: f64 = RATE as f64;
/// Fade at each side of a splice.
const SPLICE_FADE: usize = 288; // 6 ms
/// Analysis hop for speech detection.
const HOP: usize = 480; // 10 ms

/// Folder inside a bundle for files that can always be regenerated.
pub fn cache_dir(dir: &Path) -> PathBuf {
    dir.join("cache")
}

/// Run a command to the end, stopping it if `cancel` is set.
pub fn run_cancellable(mut c: Command, what: &str, cancel: &AtomicBool) -> Result<()> {
    let mut child = c
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting ffmpeg ({what})"))?;
    let errors = ffmpeg::ErrorLog::attach(&mut child);
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("ffmpeg failed ({what}): {}", errors.text());
            }
            return Ok(());
        }
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

// ---- WAV -------------------------------------------------------------------

/// Random-access reader for 16-bit PCM WAV files.
pub struct Wav {
    file: BufReader<File>,
    data_start: u64,
    pub frames: u64,
    pub channels: usize,
    pub rate: u32,
}

impl Wav {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = BufReader::new(
            File::open(path).with_context(|| format!("opening {}", path.display()))?,
        );
        let len = file.get_ref().metadata()?.len();
        let mut head = [0u8; 12];
        file.read_exact(&mut head)?;
        anyhow::ensure!(
            &head[..4] == b"RIFF" && &head[8..] == b"WAVE",
            "{} is not a WAV file",
            path.display()
        );
        let (mut channels, mut rate, mut bits) = (0usize, 0u32, 0u16);
        let mut pos = 12u64;
        loop {
            let mut chunk = [0u8; 8];
            file.seek(SeekFrom::Start(pos))?;
            file.read_exact(&mut chunk)
                .with_context(|| format!("{} has no audio data", path.display()))?;
            let size = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as u64;
            match &chunk[..4] {
                b"fmt " => {
                    let mut fmt = [0u8; 16];
                    file.read_exact(&mut fmt)?;
                    channels = u16::from_le_bytes([fmt[2], fmt[3]]) as usize;
                    rate = u32::from_le_bytes(fmt[4..8].try_into().unwrap());
                    bits = u16::from_le_bytes([fmt[14], fmt[15]]);
                }
                b"data" => {
                    anyhow::ensure!(
                        bits == 16 && channels >= 1,
                        "{} is not 16-bit PCM",
                        path.display()
                    );
                    let data_start = pos + 8;
                    // A streamed file can carry a size of 0 or 0xFFFFFFFF.
                    let size = if size == 0 || data_start + size > len {
                        len - data_start
                    } else {
                        size
                    };
                    return Ok(Self {
                        file,
                        data_start,
                        frames: size / (2 * channels as u64),
                        channels,
                        rate,
                    });
                }
                _ => {}
            }
            pos += 8 + size + (size & 1);
        }
    }

    /// Fill `out` (interleaved) with the frames starting at `frame`. Frames
    /// past the end of the file are silence.
    pub fn read(&mut self, frame: u64, out: &mut [i16]) -> Result<()> {
        out.fill(0);
        if frame >= self.frames {
            return Ok(());
        }
        let want = out.len() / self.channels;
        let have = want.min((self.frames - frame) as usize);
        let mut bytes = vec![0u8; have * self.channels * 2];
        self.file.seek(SeekFrom::Start(
            self.data_start + frame * self.channels as u64 * 2,
        ))?;
        self.file.read_exact(&mut bytes)?;
        for (o, b) in out.iter_mut().zip(bytes.as_chunks::<2>().0) {
            *o = i16::from_le_bytes(*b);
        }
        Ok(())
    }
}

/// Writes a 16-bit PCM WAV file.
pub struct WavWriter {
    file: BufWriter<File>,
    bytes: u64,
}

impl WavWriter {
    pub fn create(path: &Path, channels: u16, rate: u32) -> Result<Self> {
        let mut file = BufWriter::new(
            File::create(path).with_context(|| format!("creating {}", path.display()))?,
        );
        file.write_all(b"RIFF\0\0\0\0WAVEfmt ")?;
        file.write_all(&16u32.to_le_bytes())?;
        file.write_all(&1u16.to_le_bytes())?;
        file.write_all(&channels.to_le_bytes())?;
        file.write_all(&rate.to_le_bytes())?;
        file.write_all(&(rate * channels as u32 * 2).to_le_bytes())?;
        file.write_all(&(channels * 2).to_le_bytes())?;
        file.write_all(&16u16.to_le_bytes())?;
        file.write_all(b"data\0\0\0\0")?;
        Ok(Self { file, bytes: 0 })
    }

    pub fn write(&mut self, samples: &[i16]) -> Result<()> {
        for s in samples {
            self.file.write_all(&s.to_le_bytes())?;
        }
        self.bytes += samples.len() as u64 * 2;
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.file.flush()?;
        let mut file = self.file.into_inner().map_err(|e| e.into_error())?;
        let size = self.bytes.min(u32::MAX as u64 - 36) as u32;
        file.seek(SeekFrom::Start(4))?;
        file.write_all(&(size + 36).to_le_bytes())?;
        file.seek(SeekFrom::Start(40))?;
        file.write_all(&size.to_le_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}

// ---- voice -----------------------------------------------------------------

/// ffmpeg filter chain that turns a raw microphone track into the cleaned-up
/// voice track of a clip: aligned to the video and exactly as long.
pub fn voice_filter(offset_ms: f64, duration_ms: f64, audio: &AudioSettings) -> String {
    let mut chain: Vec<String> = Vec::new();
    if offset_ms > 0.5 {
        chain.push(format!("adelay={}:all=1", offset_ms.round() as u64));
    } else if offset_ms < -0.5 {
        chain.push(format!("atrim=start={:.4}", -offset_ms / 1000.0));
        chain.push("asetpts=PTS-STARTPTS".into());
    }
    if audio.denoise > 0.005 {
        // 6 dB is barely audible, 30 dB is heavy-handed.
        chain.push(format!(
            "afftdn=nr={:.1}:nf=-30:tn=1",
            6.0 + 24.0 * audio.denoise
        ));
    }
    let eq = &audio.eq;
    if eq.enabled {
        chain.push(format!("highpass=f={:.0}", eq.low_cut_hz));
        if eq.bass_db.abs() > 0.05 {
            chain.push(format!("equalizer=f=180:t=q:w=0.9:g={:.1}", eq.bass_db));
        }
        if eq.presence_db.abs() > 0.05 {
            chain.push(format!("equalizer=f=3200:t=q:w=1:g={:.1}", eq.presence_db));
        }
        if eq.air_db.abs() > 0.05 {
            chain.push(format!("highshelf=f=9000:g={:.1}", eq.air_db));
        }
    }
    if audio.normalize {
        chain.push("loudnorm=I=-16:TP=-1.5:LRA=11".into());
    }
    chain.push(format!("aresample={RATE}"));
    let seconds = duration_ms / 1000.0;
    chain.push(format!("apad=whole_dur={seconds:.4}"));
    chain.push(format!("atrim=end={seconds:.4}"));
    chain.join(",")
}

fn fingerprint(file: &Path, extra: &str) -> u64 {
    let mut h = DefaultHasher::new();
    if let Ok(meta) = std::fs::metadata(file) {
        meta.len().hash(&mut h);
        if let Ok(t) = meta.modified() {
            t.hash(&mut h);
        }
    }
    file.hash(&mut h);
    extra.hash(&mut h);
    h.finish()
}

/// Remove cached files with the given prefix other than `keep`.
fn prune(cache: &Path, prefix: &str, keep: &Path) {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with(prefix) && path != keep {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Decode `input` through `filter` into a cached WAV, unless it is there
/// already.
fn render_wav(
    ffmpeg: &Path,
    input: &Path,
    filter: Option<&str>,
    channels: u32,
    out: &Path,
    cancel: &AtomicBool,
) -> Result<()> {
    if out.is_file() {
        return Ok(());
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = out.with_extension("part.wav");
    let mut c = ffmpeg::command(ffmpeg);
    c.args(["-v", "error", "-y", "-i"]).arg(input).arg("-vn");
    if let Some(f) = filter {
        c.args(["-af", f]);
    }
    c.args(["-ac", &channels.to_string(), "-ar", &RATE.to_string()]);
    c.args(["-c:a", "pcm_s16le", "-f", "wav"]).arg(&part);
    let result = run_cancellable(c, "audio", cancel);
    if result.is_err() {
        ffmpeg::remove_file_patiently(&part);
        return result;
    }
    std::fs::rename(&part, out)?;
    Ok(())
}

/// The cleaned-up voice track of a recording, or `None` if it has no
/// microphone audio.
pub fn voice_track(
    ffmpeg: &Path,
    dir: &Path,
    id: &str,
    clip: &RecordingClip,
    audio: &AudioSettings,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>> {
    let Some(mic) = &clip.source.mic else {
        return Ok(None);
    };
    let input = Project::clip_dir(dir, clip).join(&mic.file);
    if !input.is_file() {
        return Ok(None);
    }
    let filter = voice_filter(mic.offset_ms, clip.source.video.duration_ms, audio);
    let cache = cache_dir(dir);
    let prefix = format!("voice-{id}-");
    let out = cache.join(format!("{prefix}{:016x}.wav", fingerprint(&input, &filter)));
    render_wav(ffmpeg, &input, Some(&filter), 1, &out, cancel)?;
    prune(&cache, &prefix, &out);
    Ok(Some(out))
}

/// The music as a stereo WAV: a built-in track or the user's file.
pub fn music_track(
    ffmpeg: &Path,
    dir: &Path,
    music: &Music,
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>> {
    let cache = cache_dir(dir);
    if let Some(id) = music.source.strip_prefix("builtin:") {
        let out = cache.join(format!("music-builtin-{id}.wav"));
        if !out.is_file() {
            let Some(samples) = crate::music::render(id) else {
                bail!("there is no built-in track called {id:?}");
            };
            std::fs::create_dir_all(&cache)?;
            let part = out.with_extension("part.wav");
            let mut w = WavWriter::create(&part, 2, crate::music::SAMPLE_RATE)?;
            let pcm: Vec<i16> = samples
                .iter()
                .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect();
            w.write(&pcm)?;
            w.finish()?;
            std::fs::rename(&part, &out)?;
        }
        return Ok(Some(out));
    }
    if music.source.is_empty() {
        return Ok(None);
    }
    let input = Path::new(&music.source);
    if !input.is_file() {
        bail!("the music file {} is missing", input.display());
    }
    let out = cache.join(format!("music-{:016x}.wav", fingerprint(input, "")));
    render_wav(ffmpeg, input, None, 2, &out, cancel)?;
    Ok(Some(out))
}

// ---- mixing ----------------------------------------------------------------

/// A stretch of output audio taken from one voice track (or silence).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    /// Index into the voice tracks, or `None` for silence.
    voice: Option<usize>,
    /// First frame inside the voice track.
    start: u64,
    frames: usize,
}

fn frames(ms: f64) -> u64 {
    (ms * RATE_F / 1000.0).round().max(0.0) as u64
}

/// Cut the timeline into the pieces of voice track that make up the output.
/// `voices[i]` says whether clip `i` has a voice track.
fn segments(project: &Project, kept: &Kept, voices: &[bool]) -> Vec<Segment> {
    let spans = project.spans();
    let mut out: Vec<Segment> = Vec::new();
    for &(ks, ke) in &kept.ranges {
        for span in &spans {
            let (a, b) = (ks.max(span.start_ms), ke.min(span.end_ms()));
            if b <= a {
                continue;
            }
            let start = frames(a - span.start_ms);
            let n = (frames(b - span.start_ms) - start) as usize;
            if n == 0 {
                continue;
            }
            let voice = voices
                .get(span.index)
                .copied()
                .unwrap_or(false)
                .then_some(span.index);
            // Two pieces of silence in a row are one piece.
            match out.last_mut() {
                Some(last) if last.voice.is_none() && voice.is_none() => last.frames += n,
                _ => out.push(Segment {
                    voice,
                    start,
                    frames: n,
                }),
            }
        }
    }
    out
}

/// Reads the spliced voice as one continuous mono stream.
struct VoiceStream<'a> {
    segments: &'a [Segment],
    tracks: &'a mut [Option<Wav>],
    gain: f32,
    /// Position: segment index and frame inside it.
    at: (usize, usize),
    scratch: Vec<i16>,
}

impl VoiceStream<'_> {
    /// Fill `out` with the next samples; returns how many were produced.
    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let mut done = 0;
        while done < out.len() {
            let Some(seg) = self.segments.get(self.at.0) else {
                break;
            };
            let n = (seg.frames - self.at.1).min(out.len() - done);
            let dst = &mut out[done..done + n];
            match seg.voice.and_then(|i| self.tracks[i].as_mut()) {
                Some(wav) => {
                    self.scratch.resize(n, 0);
                    wav.read(seg.start + self.at.1 as u64, &mut self.scratch)?;
                    for (i, (o, s)) in dst.iter_mut().zip(&self.scratch).enumerate() {
                        // Fade in and out of every piece so splices never click.
                        let pos = self.at.1 + i;
                        let edge = pos.min(seg.frames - 1 - pos);
                        let fade = ((edge + 1) as f32 / SPLICE_FADE as f32).min(1.0);
                        *o = *s as f32 / 32768.0 * self.gain * fade;
                    }
                }
                None => dst.fill(0.0),
            }
            done += n;
            self.at.1 += n;
            if self.at.1 == seg.frames {
                self.at = (self.at.0 + 1, 0);
            }
        }
        Ok(done)
    }
}

/// Music gain per analysis hop: 1 normally, `duck` while there is speech,
/// starting slightly before it and recovering slowly after.
fn duck_envelope(rms: &[f32], duck: f32) -> Vec<f32> {
    const THRESHOLD: f32 = 0.012; // about -38 dBFS
    const LOOKAHEAD: usize = 12; // 120 ms
    const HOLD: usize = 35; // 350 ms
    let mut active = vec![false; rms.len()];
    let mut hold = 0;
    for (i, &v) in rms.iter().enumerate() {
        if v > THRESHOLD {
            hold = HOLD;
            for a in &mut active[i.saturating_sub(LOOKAHEAD)..i] {
                *a = true;
            }
        }
        active[i] = hold > 0;
        hold = hold.saturating_sub(1);
    }
    let (attack, release) = (1.0 - (-1.0f32 / 6.0).exp(), 1.0 - (-1.0f32 / 50.0).exp());
    let mut gain = 1.0f32;
    active
        .iter()
        .map(|&a| {
            let target = if a { duck } else { 1.0 };
            gain += (target - gain) * if target < gain { attack } else { release };
            gain
        })
        .collect()
}

fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = 0.85;
    if x.abs() <= KNEE {
        x
    } else {
        let over = (x.abs() - KNEE) / (1.0 - KNEE);
        x.signum() * (KNEE + (1.0 - KNEE) * over.tanh())
    }
}

/// Everything the mix depends on, as a short hash. Two projects with the
/// same key produce the same soundtrack.
pub fn mix_key(project: &Project) -> String {
    let mut h = DefaultHasher::new();
    let state = serde_json::json!([
        project.cuts,
        project.audio,
        project
            .clips
            .iter()
            .map(|c| (
                &c.id,
                c.duration_ms(),
                c.recording()
                    .and_then(|r| r.source.mic.as_ref().map(|m| m.offset_ms))
            ))
            .collect::<Vec<_>>(),
    ]);
    state.to_string().hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Write the soundtrack of the finished video (stereo WAV, as long as the
/// video) to `out`. Returns `false`, writing nothing, when the project has
/// neither voice nor music. `progress` receives 0..1.
pub fn build_mix(
    dir: &Path,
    project: &Project,
    ffmpeg: &Path,
    out: &Path,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<bool> {
    let cancelled = || -> Result<()> {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled.into());
        }
        Ok(())
    };
    let audio = &project.audio;
    let with_mic = project
        .clips
        .iter()
        .filter(|c| c.recording().is_some_and(|r| r.source.mic.is_some()))
        .count();
    let mut paths: Vec<Option<PathBuf>> = Vec::new();
    let mut done = 0;
    for clip in &project.clips {
        cancelled()?;
        let path = match clip.recording() {
            Some(r) => voice_track(ffmpeg, dir, &clip.id, r, audio, cancel)?,
            None => None,
        };
        if clip.recording().is_some_and(|r| r.source.mic.is_some()) {
            done += 1;
            progress(0.7 * done as f64 / with_mic as f64);
        }
        paths.push(path);
    }
    let music = match &audio.music {
        Some(m) if m.volume > 0.0 => music_track(ffmpeg, dir, m, cancel)?.zip(Some(m)),
        _ => None,
    };
    if paths.iter().all(Option::is_none) && music.is_none() {
        return Ok(false);
    }

    let mut tracks: Vec<Option<Wav>> = paths
        .iter()
        .map(|p| p.as_deref().map(Wav::open).transpose())
        .collect::<Result<_>>()?;
    let kept = Kept::new(project.duration_ms(), &project.cuts);
    let voices: Vec<bool> = tracks.iter().map(Option::is_some).collect();
    let segs = segments(project, &kept, &voices);
    let total: usize = segs.iter().map(|s| s.frames).sum();
    let gain = audio.mic_volume as f32;

    // First pass: how loud is the voice, hop by hop.
    let mut rms = Vec::with_capacity(total / HOP + 1);
    let mut block = vec![0f32; HOP];
    let mut stream = VoiceStream {
        segments: &segs,
        tracks: &mut tracks,
        gain,
        at: (0, 0),
        scratch: Vec::new(),
    };
    loop {
        let n = stream.read(&mut block)?;
        if n == 0 {
            break;
        }
        let sum: f32 = block[..n].iter().map(|s| s * s).sum();
        rms.push((sum / n as f32).sqrt());
        if rms.len().is_multiple_of(500) {
            cancelled()?;
        }
    }
    progress(0.8);

    let mut music = match music {
        Some((path, m)) => {
            let wav = Wav::open(&path)?;
            anyhow::ensure!(wav.frames > 0, "the music file is empty");
            let duck = if m.duck {
                10f32.powf(m.duck_db as f32 / 20.0)
            } else {
                1.0
            };
            Some((wav, m, duck_envelope(&rms, duck)))
        }
        None => None,
    };

    // Second pass: voice plus music.
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = WavWriter::create(out, 2, RATE)?;
    let mut stream = VoiceStream {
        segments: &segs,
        tracks: &mut tracks,
        gain,
        at: (0, 0),
        scratch: Vec::new(),
    };
    let mut pcm = vec![0i16; HOP * 2];
    let mut tune = vec![0i16; HOP * 2];
    let mut pos = 0usize;
    let mut hop = 0usize;
    loop {
        let n = stream.read(&mut block)?;
        if n == 0 {
            break;
        }
        if let Some((wav, _, _)) = &mut music {
            // Loop the track: read up to the end, then wrap.
            let at = pos as u64 % wav.frames;
            let first = n.min((wav.frames - at) as usize);
            wav.read(at, &mut tune[..first * 2])?;
            if first < n {
                wav.read(0, &mut tune[first * 2..n * 2])?;
            }
        }
        for i in 0..n {
            let (mut l, mut r) = (block[i], block[i]);
            if let Some((_, m, env)) = &music {
                let t = pos + i;
                let fade_in = if m.fade_in_ms > 0.0 {
                    (t as f64 / (m.fade_in_ms * RATE_F / 1000.0)).min(1.0)
                } else {
                    1.0
                };
                let fade_out = if m.fade_out_ms > 0.0 {
                    ((total - t) as f64 / (m.fade_out_ms * RATE_F / 1000.0)).min(1.0)
                } else {
                    1.0
                };
                // Glide between the per-hop gains.
                let a = env[hop];
                let b = env.get(hop + 1).copied().unwrap_or(a);
                let duck = a + (b - a) * i as f32 / HOP as f32;
                let g = m.volume as f32 * duck * (fade_in * fade_out) as f32 / 32768.0;
                l += tune[i * 2] as f32 * g;
                r += tune[i * 2 + 1] as f32 * g;
            }
            pcm[i * 2] = (soft_clip(l) * 32767.0) as i16;
            pcm[i * 2 + 1] = (soft_clip(r) * 32767.0) as i16;
        }
        writer.write(&pcm[..n * 2])?;
        pos += n;
        hop += 1;
        if hop.is_multiple_of(500) {
            cancelled()?;
            progress(0.8 + 0.2 * pos as f64 / total.max(1) as f64);
        }
    }
    writer.finish()?;
    progress(1.0);
    Ok(true)
}

/// The soundtrack for the editor preview, cached inside the bundle. Returns
/// its path, or `None` when the project is silent.
pub fn preview_mix(
    dir: &Path,
    project: &Project,
    ffmpeg: &Path,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<Option<PathBuf>> {
    let cache = cache_dir(dir);
    let out = cache.join(format!("mix-{}.wav", mix_key(project)));
    if out.is_file() {
        progress(1.0);
        return Ok(Some(out));
    }
    let part = cache.join("mix.part");
    let made = build_mix(dir, project, ffmpeg, &part, progress, cancel);
    match made {
        Ok(true) => {
            std::fs::rename(&part, &out)?;
            prune(&cache, "mix-", &out);
            Ok(Some(out))
        }
        Ok(false) => Ok(None),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

/// Stretches of the timeline where nobody is speaking, at least `min_ms`
/// long, found by listening to the cleaned-up voice tracks.
pub fn detect_silences(
    dir: &Path,
    project: &Project,
    ffmpeg: &Path,
    min_ms: f64,
    cancel: &AtomicBool,
) -> Result<Vec<(f64, f64)>> {
    let mut out = Vec::new();
    for span in project.spans() {
        let clip = &project.clips[span.index];
        let Some(rec) = clip.recording() else {
            continue;
        };
        let Some(path) = voice_track(ffmpeg, dir, &clip.id, rec, &project.audio, cancel)? else {
            continue;
        };
        let mut wav = Wav::open(&path)?;
        let hops = (wav.frames as usize).div_ceil(HOP);
        let mut rms = Vec::with_capacity(hops);
        let mut buf = vec![0i16; HOP * wav.channels];
        for i in 0..hops {
            wav.read((i * HOP) as u64, &mut buf)?;
            let sum: f64 = buf.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
            rms.push((sum / buf.len() as f64).sqrt());
        }
        out.extend(
            silent_runs(&rms, min_ms)
                .into_iter()
                .map(|(a, b)| (span.start_ms + a, (span.start_ms + b).min(span.end_ms()))),
        );
    }
    Ok(out)
}

/// Runs of quiet hops at least `min_ms` long, as `(start_ms, end_ms)`.
/// "Quiet" is relative to how loud the speech in the track is.
fn silent_runs(rms: &[f64], min_ms: f64) -> Vec<(f64, f64)> {
    let hop_ms = HOP as f64 * 1000.0 / RATE_F;
    let mut sorted: Vec<f64> = rms.to_vec();
    sorted.sort_by(f64::total_cmp);
    let speech = sorted.get(sorted.len() * 9 / 10).copied().unwrap_or(0.0);
    // 30 dB below speech, but never above -40 dBFS or below -60 dBFS.
    let threshold = (speech * 0.0316).clamp(0.001, 0.01);
    let mut out = Vec::new();
    let mut start = None;
    for (i, &v) in rms.iter().chain(std::iter::once(&f64::MAX)).enumerate() {
        match (v < threshold, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if (i - s) as f64 * hop_ms >= min_ms {
                    out.push((s as f64 * hop_ms, i as f64 * hop_ms));
                }
                start = None;
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Card, Clip, ClipBody, Cut, Transition, VoiceEq};
    use crate::testutil::project;

    #[test]
    fn voice_filter_aligns_cleans_and_fits() {
        let mut a = AudioSettings::default();
        assert_eq!(
            voice_filter(0.0, 2500.0, &a),
            "aresample=48000,apad=whole_dur=2.5000,atrim=end=2.5000"
        );
        assert!(voice_filter(120.0, 1000.0, &a).starts_with("adelay=120:all=1,aresample"));
        assert!(voice_filter(-35.0, 1000.0, &a)
            .starts_with("atrim=start=0.0350,asetpts=PTS-STARTPTS,aresample"));
        a.denoise = 0.5;
        a.normalize = true;
        a.eq = VoiceEq {
            enabled: true,
            low_cut_hz: 90.0,
            bass_db: 0.0,
            presence_db: 4.0,
            air_db: -2.0,
        };
        assert_eq!(
            voice_filter(0.0, 1000.0, &a),
            "afftdn=nr=18.0:nf=-30:tn=1,highpass=f=90,equalizer=f=3200:t=q:w=1:g=4.0,\
             highshelf=f=9000:g=-2.0,loudnorm=I=-16:TP=-1.5:LRA=11,aresample=48000,\
             apad=whole_dur=1.0000,atrim=end=1.0000"
        );
    }

    #[test]
    fn wav_round_trip_and_reads_past_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.wav");
        let mut w = WavWriter::create(&path, 2, RATE).unwrap();
        let data: Vec<i16> = (0..2000).map(|i| (i * 7 % 3000) as i16 - 1500).collect();
        w.write(&data).unwrap();
        w.finish().unwrap();
        let mut wav = Wav::open(&path).unwrap();
        assert_eq!((wav.frames, wav.channels, wav.rate), (1000, 2, RATE));
        let mut buf = vec![0i16; 20];
        wav.read(995, &mut buf).unwrap();
        assert_eq!(&buf[..10], &data[1990..]);
        assert!(buf[10..].iter().all(|&s| s == 0));
        wav.read(5000, &mut buf).unwrap();
        assert!(buf.iter().all(|&s| s == 0));
        assert!(Wav::open(&dir.path().join("missing.wav")).is_err());
        std::fs::write(&path, b"not audio at all").unwrap();
        assert!(Wav::open(&path).is_err());
    }

    #[test]
    fn segments_follow_cuts_and_clips() {
        let mut p = project(100, 100, 4000.0);
        p.clips.push(Clip {
            id: "card".into(),
            transition: Transition::default(),
            body: ClipBody::Card(Card {
                duration_ms: 1000.0,
                ..Card::default()
            }),
        });
        p.cuts = vec![Cut {
            start_ms: 1000.0,
            end_ms: 3000.0,
        }];
        let kept = Kept::new(p.duration_ms(), &p.cuts);
        let s = segments(&p, &kept, &[true, false]);
        assert_eq!(
            s,
            vec![
                Segment {
                    voice: Some(0),
                    start: 0,
                    frames: 48_000
                },
                Segment {
                    voice: Some(0),
                    start: 144_000,
                    frames: 48_000
                },
                Segment {
                    voice: None,
                    start: 0,
                    frames: 48_000
                },
            ]
        );
        // Total length is the output length.
        assert_eq!(s.iter().map(|x| x.frames).sum::<usize>(), 144_000);
        // No voice anywhere: one stretch of silence.
        assert_eq!(segments(&p, &kept, &[false, false]).len(), 1);
    }

    #[test]
    fn music_ducks_before_speech_and_recovers_after() {
        let mut rms = vec![0.0f32; 400];
        rms[100..200].fill(0.2);
        let env = duck_envelope(&rms, 0.2);
        assert!(env[50] > 0.99);
        // Already most of the way down when speech starts.
        assert!(env[100] < 0.5, "{}", env[100]);
        assert!(env[150] < 0.21);
        // Held just after speech, back up later.
        assert!(env[215] < 0.3);
        assert!(env[399] > 0.9, "{}", env[399]);
        assert!(env.iter().all(|g| (0.2..=1.0).contains(g)));
        // Ducking off: untouched.
        assert!(duck_envelope(&rms, 1.0).iter().all(|g| *g == 1.0));
    }

    #[test]
    fn silence_is_relative_to_speech() {
        let mut rms = vec![0.0002f64; 1000];
        rms[100..300].fill(0.1);
        rms[450..460].fill(0.1);
        rms[700..1000].fill(0.1);
        let runs = silent_runs(&rms, 1000.0);
        assert_eq!(
            runs,
            vec![(0.0, 1000.0), (3000.0, 4500.0), (4600.0, 7000.0)]
        );
        assert_eq!(silent_runs(&rms, 2000.0).len(), 1);
        assert!(silent_runs(&[], 500.0).is_empty());
        // A noisy room: the floor is well above digital silence.
        let mut noisy = vec![0.004f64; 600];
        noisy[..300].fill(0.2);
        assert_eq!(silent_runs(&noisy, 1000.0), vec![(3000.0, 6000.0)]);
    }

    #[test]
    fn soft_clip_stays_in_range() {
        assert_eq!(soft_clip(0.5), 0.5);
        assert!(soft_clip(3.0) <= 1.0 && soft_clip(3.0) > 0.85);
        assert!(soft_clip(-3.0) >= -1.0);
    }

    #[test]
    fn mix_key_tracks_what_matters() {
        let mut p = project(100, 100, 4000.0);
        let a = mix_key(&p);
        p.frame.padding = 0.2;
        assert_eq!(mix_key(&p), a);
        p.audio.denoise = 0.4;
        let b = mix_key(&p);
        assert_ne!(a, b);
        p.cuts.push(Cut {
            start_ms: 0.0,
            end_ms: 500.0,
        });
        assert_ne!(mix_key(&p), b);
    }
}
