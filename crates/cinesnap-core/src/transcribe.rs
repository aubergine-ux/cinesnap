//! Speech to text with whisper.cpp, running on this machine. The only thing
//! that ever touches the network is the one-time model download.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{bail, Context, Result};
use serde::Serialize;

use crate::audio;
use crate::export::Cancelled;
use crate::ffmpeg;
use crate::project::{AudioSettings, Project, Transcript, Word};

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// Download size.
    pub megabytes: u32,
    pub english_only: bool,
}

pub const MODELS: [ModelInfo; 5] = [
    ModelInfo {
        id: "base.en",
        label: "Base (English)",
        hint: "Good accuracy, fast. The default.",
        megabytes: 148,
        english_only: true,
    },
    ModelInfo {
        id: "tiny.en",
        label: "Tiny (English)",
        hint: "Fastest, makes more mistakes.",
        megabytes: 78,
        english_only: true,
    },
    ModelInfo {
        id: "small.en",
        label: "Small (English)",
        hint: "Most accurate of the three, several times slower.",
        megabytes: 488,
        english_only: true,
    },
    ModelInfo {
        id: "base",
        label: "Base (any language)",
        hint: "For speech that is not English.",
        megabytes: 148,
        english_only: false,
    },
    ModelInfo {
        id: "small",
        label: "Small (any language)",
        hint: "More accurate for other languages, slower.",
        megabytes: 488,
        english_only: false,
    },
];

pub fn model(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// Where a model lives once downloaded.
pub fn model_path(models_dir: &Path, id: &str) -> PathBuf {
    models_dir.join(format!("ggml-{id}.bin"))
}

/// Whether speech recognition was compiled into this build.
pub fn available() -> bool {
    cfg!(feature = "whisper")
}

/// Download a model into `models_dir` unless it is already there.
/// `progress` receives 0..1.
pub fn download_model(
    models_dir: &Path,
    id: &str,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<PathBuf> {
    let info = model(id).with_context(|| format!("there is no model called {id:?}"))?;
    let out = model_path(models_dir, id);
    if out.is_file() {
        return Ok(out);
    }
    std::fs::create_dir_all(models_dir)?;
    let url = format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{id}.bin");
    let mut response = ureq::get(&url)
        .call()
        .with_context(|| format!("downloading the {} model", info.label))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(info.megabytes as u64 * 1_000_000);
    let part = out.with_extension("part");
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&part)?;
        let mut reader = response.body_mut().as_reader();
        let mut buf = vec![0u8; 1 << 16];
        let mut done = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(Cancelled.into());
            }
            let n = reader
                .read(&mut buf)
                .context("the download was interrupted")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            done += n as u64;
            progress((done as f64 / total as f64).min(1.0));
        }
        file.sync_all()?;
        // A proxy error page is not a model.
        anyhow::ensure!(
            done > 1_000_000,
            "the download was incomplete ({done} bytes)"
        );
        Ok(())
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &out)?;
    Ok(out)
}

/// The clip's microphone track as 16 kHz mono samples, aligned to the video.
fn speech_samples(
    ffmpeg: &Path,
    mic: &Path,
    offset_ms: f64,
    duration_ms: f64,
    cancel: &AtomicBool,
) -> Result<Vec<f32>> {
    let filter = audio::voice_filter(offset_ms, duration_ms, &AudioSettings::default());
    let mut c = ffmpeg::command(ffmpeg);
    c.args(["-v", "error", "-i"]).arg(mic);
    c.args(["-vn", "-af", &filter, "-ac", "1", "-ar", "16000"]);
    c.args(["-f", "f32le", "-"]);
    let mut child = c
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")?;
    let errors = ffmpeg::ErrorLog::attach(&mut child);
    let mut out = child.stdout.take().unwrap();
    let mut bytes = Vec::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Cancelled.into());
        }
        let n = out.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
    }
    if !child.wait()?.success() {
        bail!("could not read the microphone track: {}", errors.text());
    }
    Ok(bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect())
}

/// Join the pieces whisper produces into words. A piece that does not start
/// with a space continues the word before it.
pub fn merge_pieces(pieces: &[(f64, f64, String)]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    for (t0, t1, text) in pieces {
        let trimmed = text.trim();
        // Sound descriptions such as [BLANK_AUDIO] or (music).
        if trimmed.is_empty()
            || (trimmed.starts_with('[') && trimmed.ends_with(']'))
            || (trimmed.starts_with('(') && trimmed.ends_with(')'))
        {
            continue;
        }
        let continues = !text.starts_with(' ')
            && words.last().is_some_and(|w| t0 - w.t1 < 200.0)
            // Scripts without spaces: every piece is its own word.
            && trimmed.chars().all(|c| c.is_ascii() || c.is_alphabetic() && c.is_lowercase());
        match words.last_mut() {
            Some(last) if continues => {
                last.text.push_str(trimmed);
                last.t1 = t1.max(last.t1);
            }
            _ => words.push(Word {
                t0: *t0,
                t1: t1.max(*t0),
                text: trimmed.to_owned(),
            }),
        }
    }
    words
}

#[cfg(feature = "whisper")]
fn recognise(
    model: &Path,
    samples: &[f32],
    language: Option<&str>,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<(Vec<Word>, String)> {
    use std::sync::atomic::AtomicI32;
    use std::sync::Arc;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    static QUIET: std::sync::Once = std::sync::Once::new();
    QUIET.call_once(whisper_rs::install_logging_hooks);

    let percent = Arc::new(AtomicI32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let worker = scope.spawn(|| -> Result<(Vec<Word>, String)> {
            let finished = scopeguard(&done);
            let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
                .map_err(|e| anyhow::anyhow!("could not load the speech model: {e}"))?;
            let mut state = ctx
                .create_state()
                .map_err(|e| anyhow::anyhow!("could not start the speech model: {e}"))?;
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
            params.set_n_threads(threads as i32);
            params.set_language(language);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // One word per segment, with its own timestamps.
            params.set_token_timestamps(true);
            params.set_split_on_word(true);
            params.set_max_len(1);
            // Whisper tends to tidy speech up; this keeps the hesitations in,
            // so they can be found and cut.
            params.set_initial_prompt(
                "Umm, let me think like, hmm... Okay, uh, here's what I'm thinking.",
            );
            let p = percent.clone();
            params.set_progress_callback_safe(move |v: i32| p.store(v, Ordering::Relaxed));
            let s = stop.clone();
            params.set_abort_callback_safe(move || s.load(Ordering::Relaxed));
            state
                .full(params, samples)
                .map_err(|e| anyhow::anyhow!("speech recognition failed: {e}"))?;
            let mut pieces = Vec::new();
            for segment in state.as_iter() {
                let text = segment
                    .to_str_lossy()
                    .map(|s| s.into_owned())
                    .unwrap_or_default();
                // Timestamps are in hundredths of a second.
                pieces.push((
                    segment.start_timestamp() as f64 * 10.0,
                    segment.end_timestamp() as f64 * 10.0,
                    text,
                ));
            }
            let lang = whisper_rs::get_lang_str(state.full_lang_id_from_state())
                .unwrap_or("en")
                .to_owned();
            drop(finished);
            Ok((merge_pieces(&pieces), lang))
        });
        while !done.load(Ordering::Relaxed) && !worker.is_finished() {
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
            }
            progress(percent.load(Ordering::Relaxed) as f64 / 100.0);
            std::thread::sleep(std::time::Duration::from_millis(80));
        }
        worker
            .join()
            .unwrap_or_else(|_| bail!("speech recognition crashed"))
    });
    if cancel.load(Ordering::Relaxed) {
        return Err(Cancelled.into());
    }
    result
}

#[cfg(feature = "whisper")]
fn scopeguard(flag: &AtomicBool) -> impl Drop + '_ {
    struct Guard<'a>(&'a AtomicBool);
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    Guard(flag)
}

#[cfg(not(feature = "whisper"))]
fn recognise(
    _model: &Path,
    _samples: &[f32],
    _language: Option<&str>,
    _progress: &mut dyn FnMut(f64),
    _cancel: &AtomicBool,
) -> Result<(Vec<Word>, String)> {
    bail!("this build of Cinesnap was made without speech recognition")
}

/// Transcribe every recording on the timeline. Word times are timeline
/// times. `language` is a code such as `en`, or `None` to detect it.
pub fn transcribe(
    dir: &Path,
    project: &Project,
    ffmpeg: &Path,
    model: &Path,
    language: Option<&str>,
    progress: &mut dyn FnMut(f64),
    cancel: &AtomicBool,
) -> Result<Transcript> {
    anyhow::ensure!(
        model.is_file(),
        "the speech model {} is missing",
        model.display()
    );
    let clips: Vec<_> = project
        .spans()
        .into_iter()
        .filter_map(|span| {
            let rec = project.clips[span.index].recording()?;
            let mic = rec.source.mic.as_ref()?;
            let path = Project::clip_dir(dir, rec).join(&mic.file);
            path.is_file().then_some((span, path, mic.offset_ms))
        })
        .collect();
    if clips.is_empty() {
        bail!("there is no microphone audio to transcribe");
    }
    // English-only models cannot detect a language (and report nonsense
    // when asked to).
    let english_only = model
        .file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".en.bin"));
    let language = if english_only { Some("en") } else { language };
    let total: f64 = clips.iter().map(|(s, _, _)| s.duration_ms).sum();
    let mut before = 0.0;
    let mut transcript = Transcript::default();
    for (span, mic, offset_ms) in clips {
        let samples = speech_samples(ffmpeg, &mic, offset_ms, span.duration_ms, cancel)?;
        let share = span.duration_ms / total.max(1.0);
        let (words, lang) = recognise(
            model,
            &samples,
            language,
            &mut |f| progress((before + f * share).min(1.0)),
            cancel,
        )?;
        before += share;
        if transcript.language.is_empty() {
            transcript.language = language.map_or(lang, str::to_owned);
        }
        transcript.words.extend(words.into_iter().map(|w| Word {
            t0: (span.start_ms + w.t0).min(span.end_ms()),
            t1: (span.start_ms + w.t1).min(span.end_ms()),
            text: w.text,
        }));
    }
    progress(1.0);
    Ok(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_join_into_words() {
        let piece = |t0: f64, t1: f64, s: &str| (t0, t1, s.to_owned());
        let words = merge_pieces(&[
            piece(0.0, 300.0, " Hello"),
            piece(300.0, 320.0, ","),
            piece(400.0, 700.0, " every"),
            piece(700.0, 900.0, "one"),
            piece(900.0, 950.0, "."),
            piece(1000.0, 3000.0, " [BLANK_AUDIO]"),
            piece(3000.0, 3200.0, " Um"),
            piece(9000.0, 9100.0, "late"),
        ]);
        let text: Vec<_> = words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(text, ["Hello,", "everyone.", "Um", "late"]);
        assert_eq!((words[1].t0, words[1].t1), (400.0, 950.0));
        assert!(merge_pieces(&[]).is_empty());
    }

    #[test]
    fn models_are_listed_and_located() {
        assert_eq!(MODELS[0].id, "base.en");
        assert!(model("nope").is_none());
        assert_eq!(
            model_path(Path::new("/m"), "tiny.en"),
            Path::new("/m/ggml-tiny.en.bin")
        );
        let cancel = AtomicBool::new(false);
        assert!(download_model(Path::new("/nonexistent"), "nope", &mut |_| {}, &cancel).is_err());
    }
}
