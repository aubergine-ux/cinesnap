//! Writing live audio to a WAV file when the source can go quiet.
//!
//! Loopback sources (WASAPI, Core Audio taps, an app's PipeWire stream)
//! deliver nothing at all while nothing plays. Written back to back, the
//! chunks would drift ahead of the video, so the gaps are filled with
//! silence from the capture instants.

use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use cinesnap_core::audio::WavWriter;

/// Gaps shorter than this are jitter in the callback timing, not silence.
const GAP_MS: f64 = 40.0;

pub struct Tape {
    writer: WavWriter,
    channels: usize,
    rate: u32,
    first: Option<Instant>,
    frames: u64,
}

impl Tape {
    pub fn create(path: &Path, channels: u16, rate: u32) -> Result<Self> {
        Ok(Self {
            writer: WavWriter::create(path, channels, rate)?,
            channels: channels.max(1) as usize,
            rate,
            first: None,
            frames: 0,
        })
    }

    /// When the first sample was captured.
    pub fn first_sample(&self) -> Option<Instant> {
        self.first
    }

    fn frames_at(&self, at: Instant) -> u64 {
        self.first.map_or(0, |f| {
            (at.saturating_duration_since(f).as_secs_f64() * self.rate as f64) as u64
        })
    }

    fn silence(&mut self, frames: u64) -> Result<()> {
        let block = vec![0i16; 4096 * self.channels];
        let mut left = frames;
        while left > 0 {
            let n = left.min(4096) as usize;
            self.writer.write(&block[..n * self.channels])?;
            left -= n as u64;
        }
        self.frames += frames;
        Ok(())
    }

    /// `samples` are interleaved, the first one captured at `at`.
    pub fn push(&mut self, at: Instant, samples: &[f32]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        self.first.get_or_insert(at);
        let due = self.frames_at(at);
        if due as f64 > self.frames as f64 + GAP_MS * self.rate as f64 / 1000.0 {
            self.silence(due - self.frames)?;
        }
        let pcm: Vec<i16> = samples
            .iter()
            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect();
        self.writer.write(&pcm)?;
        self.frames += (samples.len() / self.channels) as u64;
        Ok(())
    }

    /// Pad with silence up to `until` and close the file.
    pub fn finish(mut self, until: Instant) -> Result<()> {
        let due = self.frames_at(until);
        if due > self.frames {
            self.silence(due - self.frames)?;
        }
        self.writer.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cinesnap_core::audio::Wav;
    use std::time::Duration;

    fn read(path: &Path) -> Vec<i16> {
        let mut wav = Wav::open(path).unwrap();
        let mut out = vec![0i16; wav.frames as usize * wav.channels];
        wav.read(0, &mut out).unwrap();
        out
    }

    #[test]
    fn silence_fills_the_gaps_between_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.wav");
        let t0 = Instant::now();
        let mut tape = Tape::create(&path, 1, 1000).unwrap();
        tape.push(t0, &[0.5; 100]).unwrap();
        // Nothing for 400 ms, then another 100 ms.
        tape.push(t0 + Duration::from_millis(500), &[0.5; 100])
            .unwrap();
        tape.finish(t0 + Duration::from_millis(1000)).unwrap();
        let s = read(&path);
        assert_eq!(s.len(), 1000);
        assert!(s[..100].iter().all(|&v| v > 16000));
        assert!(s[100..500].iter().all(|&v| v == 0));
        assert!(s[500..600].iter().all(|&v| v > 16000));
        assert!(s[600..].iter().all(|&v| v == 0));
    }

    #[test]
    fn jitter_is_not_mistaken_for_a_gap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("b.wav");
        let t0 = Instant::now();
        let mut tape = Tape::create(&path, 2, 1000).unwrap();
        // 10 ms chunks arriving up to 20 ms late or early.
        for (i, jitter) in [0i64, 15, -5, 20, 0].into_iter().enumerate() {
            let at = t0 + Duration::from_millis((i as i64 * 10 + 20 + jitter) as u64);
            tape.push(at, &[0.25; 20]).unwrap();
        }
        tape.finish(t0).unwrap();
        assert_eq!(read(&path).len(), 5 * 20);
    }

    #[test]
    fn the_first_chunk_sets_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.wav");
        let t0 = Instant::now();
        let mut tape = Tape::create(&path, 1, 1000).unwrap();
        assert!(tape.first_sample().is_none());
        tape.push(t0 + Duration::from_millis(250), &[0.1; 10])
            .unwrap();
        assert_eq!(tape.first_sample(), Some(t0 + Duration::from_millis(250)));
        tape.finish(t0 + Duration::from_millis(300)).unwrap();
        assert_eq!(read(&path).len(), 50);
    }
}
