//! Built-in background music.
//!
//! The tracks are synthesised from code, so no audio files ship with the app.
//! Every track is one loop: notes, echoes and reverb tails wrap around the end
//! of the buffer into its start, so it can be repeated without a seam.

use std::f32::consts::{FRAC_PI_4, TAU};
use std::sync::OnceLock;

pub const SAMPLE_RATE: u32 = 48_000;

const SR: f32 = SAMPLE_RATE as f32;
const TARGET_RMS: f32 = 0.1;

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub mood: &'static str,
    pub bpm: u32,
    pub duration_ms: u32,
}

static TRACKS: [TrackInfo; 5] = [
    TrackInfo {
        id: "open-sky",
        name: "Open Sky",
        mood: "Calm, spacious",
        bpm: 60,
        duration_ms: 32_000,
    },
    TrackInfo {
        id: "slow-coffee",
        name: "Slow Coffee",
        mood: "Mellow, laid-back",
        bpm: 80,
        duration_ms: 48_000,
    },
    TrackInfo {
        id: "bright-side",
        name: "Bright Side",
        mood: "Upbeat, light",
        bpm: 120,
        duration_ms: 32_000,
    },
    TrackInfo {
        id: "wooden-steps",
        name: "Wooden Steps",
        mood: "Warm, minimal",
        bpm: 96,
        duration_ms: 40_000,
    },
    TrackInfo {
        id: "deep-focus",
        name: "Deep Focus",
        mood: "Steady, focused",
        bpm: 100,
        duration_ms: 38_400,
    },
];

/// The built-in tracks, in display order.
pub fn tracks() -> &'static [TrackInfo] {
    &TRACKS
}

/// One loop of the track as interleaved stereo f32 at SAMPLE_RATE, or None for an unknown id.
pub fn render(id: &str) -> Option<Vec<f32>> {
    let (index, info) = TRACKS.iter().enumerate().find(|(_, t)| t.id == id)?;
    let mut mix = Mix::new(info, 0x5eed_0000 + index as u64);
    let fx = match index {
        0 => open_sky(&mut mix),
        1 => slow_coffee(&mut mix),
        2 => bright_side(&mut mix),
        3 => wooden_steps(&mut mix),
        _ => deep_focus(&mut mix),
    };
    Some(mix.master(&fx))
}

// ---------------------------------------------------------------- helpers

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u32 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 32) as u32
    }

    /// Uniform in [0, 1).
    fn uni(&mut self) -> f32 {
        (self.next() >> 8) as f32 / 16_777_216.0
    }

    /// Uniform in [-1, 1).
    fn bip(&mut self) -> f32 {
        self.uni() * 2.0 - 1.0
    }
}

const TAB: usize = 4096;

fn sine() -> &'static [f32; TAB + 1] {
    static T: OnceLock<Box<[f32; TAB + 1]>> = OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([0.0f32; TAB + 1]);
        for (i, v) in t.iter_mut().enumerate() {
            *v = (i as f64 / TAB as f64 * std::f64::consts::TAU).sin() as f32;
        }
        t
    })
}

/// Sine of a phase given in cycles (must be above -16).
#[inline(always)]
fn fsin(tab: &[f32; TAB + 1], cycles: f32) -> f32 {
    let x = (cycles + 16.0) * TAB as f32;
    let i = x as u32;
    let f = x - i as f32;
    let k = i as usize & (TAB - 1);
    let a = tab[k];
    a + (tab[k + 1] - a) * f
}

/// Saw with the step at the wrap smoothed out (polyBLEP).
#[inline(always)]
fn saw(p: f32, inc: f32) -> f32 {
    let mut s = 2.0 * p - 1.0;
    if p < inc {
        let t = p / inc;
        s -= t + t - t * t - 1.0;
    } else if p > 1.0 - inc {
        let t = (p - 1.0) / inc;
        s -= t * t + t + t + 1.0;
    }
    s
}

#[inline(always)]
fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Equal temperament, A4 (note 69) = 440 Hz.
fn hz(note: i32) -> f32 {
    440.0 * 2f32.powf((note - 69) as f32 / 12.0)
}

/// Per-sample multiplier for an exponential decay with the given time constant.
fn decay(tau: f32) -> f32 {
    (-1.0 / (tau * SR)).exp()
}

/// One-pole low-pass coefficient.
fn pole(freq: f32) -> f32 {
    1.0 - (-TAU * freq / SR).exp()
}

fn limit(x: f32) -> f32 {
    const KNEE: f32 = 0.5;
    const CEIL: f32 = 0.86;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        (KNEE + (CEIL - KNEE) * ((a - KNEE) / (CEIL - KNEE)).tanh()).copysign(x)
    }
}

fn rms(buf: &[f32]) -> f32 {
    (buf.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>() / buf.len().max(1) as f64).sqrt()
        as f32
}

/// Runs a stateful effect over the tail of the loop and then the whole loop, so
/// its state at frame 0 is what the end of the previous repeat left behind.
fn looped(n: usize, warm: usize, mut f: impl FnMut(usize, bool)) {
    let warm = warm.min(n);
    for i in n - warm..n {
        f(i, false);
    }
    for i in 0..n {
        f(i, true);
    }
}

// ---------------------------------------------------------------- effects

struct Fx {
    /// Reverb comb feedback (decay length) and damping.
    rev_fb: f32,
    rev_damp: f32,
    rev_level: f32,
    /// Ping-pong delay time in beats.
    dly_beats: f64,
    dly_fb: f32,
    dly_level: f32,
    /// Master low-pass corner.
    tone: f32,
    /// How much the range above 1.5 kHz is lifted.
    lift: f32,
}

/// Depth of the reverb's delay modulation, in samples.
const WOBBLE: f32 = 6.0;

struct Comb {
    buf: Vec<f32>,
    delay: f32,
    pos: usize,
    lp: f32,
}

impl Comb {
    fn new(len: usize) -> Self {
        Comb {
            buf: vec![0.0; len + WOBBLE as usize + 2],
            delay: len as f32,
            pos: 0,
            lp: 0.0,
        }
    }

    /// `off` moves the read point by a few samples, which keeps the comb from
    /// ringing at fixed pitches under held notes.
    #[inline(always)]
    fn tick(&mut self, x: f32, fb: f32, damp: f32, off: f32) -> f32 {
        let len = self.buf.len();
        let d = self.delay + off;
        let whole = d as usize;
        let frac = d - whole as f32;
        let mut r = self.pos + len - whole;
        if r >= len {
            r -= len;
        }
        let before = if r == 0 { len - 1 } else { r - 1 };
        let y = self.buf[r] + (self.buf[before] - self.buf[r]) * frac;
        self.lp = y + (self.lp - y) * damp;
        self.buf[self.pos] = x + self.lp * fb;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn new(len: usize) -> Self {
        Allpass {
            buf: vec![0.0; len],
            pos: 0,
        }
    }

    #[inline(always)]
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = x + b * 0.5;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        b - x
    }
}

/// Schroeder reverb: eight damped, gently modulated combs into four allpasses per channel.
struct Reverb {
    combs: [Vec<Comb>; 2],
    aps: [Vec<Allpass>; 2],
    fb: f32,
    damp: f32,
    gain: f32,
}

impl Reverb {
    fn new(fb: f32, damp: f32) -> Self {
        const COMBS: [usize; 8] = [607, 647, 695, 739, 773, 811, 847, 881];
        const APS: [usize; 4] = [302, 241, 185, 122];
        let side = |spread: usize| {
            (
                COMBS
                    .iter()
                    .map(|l| Comb::new(l + spread))
                    .collect::<Vec<_>>(),
                APS.iter()
                    .map(|l| Allpass::new(l + spread))
                    .collect::<Vec<_>>(),
            )
        };
        let (cl, al) = side(0);
        let (cr, ar) = side(12);
        Reverb {
            combs: [cl, cr],
            aps: [al, ar],
            fb,
            damp,
            gain: ((1.0 - fb * fb) / 8.0).sqrt(),
        }
    }

    #[inline(always)]
    fn tick(&mut self, l: f32, r: f32, lfo: (f32, f32)) -> (f32, f32) {
        // Each comb follows the same slow wobble at its own phase.
        const PHASES: [(f32, f32); 8] = [
            (1.0, 0.0),
            (-0.707, 0.707),
            (0.0, -1.0),
            (0.707, 0.707),
            (-1.0, 0.0),
            (0.707, -0.707),
            (0.0, 1.0),
            (-0.707, -0.707),
        ];
        let ins = [0.75 * l + 0.25 * r, 0.25 * l + 0.75 * r];
        let mut out = [0.0f32; 2];
        for ch in 0..2 {
            let (a, b) = if ch == 0 { lfo } else { (-lfo.1, lfo.0) };
            let mut acc = 0.0;
            for (c, (ca, cb)) in self.combs[ch].iter_mut().zip(PHASES) {
                acc += c.tick(
                    ins[ch],
                    self.fb,
                    self.damp,
                    WOBBLE * 0.5 * (1.0 + ca * a + cb * b).clamp(0.0, 2.0),
                );
            }
            for a in self.aps[ch].iter_mut() {
                acc = a.tick(acc);
            }
            out[ch] = acc * self.gain;
        }
        (0.8 * out[0] + 0.2 * out[1], 0.2 * out[0] + 0.8 * out[1])
    }
}

// ---------------------------------------------------------------- mixer and instruments

/// Send levels: [reverb, delay].
type Sends = [f32; 2];

struct Mix {
    n: usize,
    bpm: f64,
    /// Samples per beat.
    spb: f64,
    /// Timing looseness in beats.
    loose: f64,
    rng: Rng,
    dry: Vec<f32>,
    rev: Vec<f32>,
    dly: Vec<f32>,
}

impl Mix {
    fn new(info: &TrackInfo, seed: u64) -> Self {
        let n = info.duration_ms as usize * (SAMPLE_RATE as usize / 1000);
        let bpm = info.bpm as f64;
        Mix {
            n,
            bpm,
            spb: SR as f64 * 60.0 / bpm,
            loose: 0.004 * bpm / 60.0,
            rng: Rng(seed),
            dry: vec![0.0; n * 2],
            rev: vec![0.0; n * 2],
            dly: vec![0.0; n * 2],
        }
    }

    fn secs(&self, beats: f64) -> f32 {
        (beats * 60.0 / self.bpm) as f32
    }

    fn human(&mut self, beat: f64, vel: f32) -> (f64, f32) {
        (
            beat + self.rng.bip() as f64 * self.loose,
            vel * (1.0 + 0.1 * self.rng.bip()),
        )
    }

    /// Adds a mono voice at the given beat, wrapping around the loop end.
    #[inline(always)]
    fn voice(
        &mut self,
        beat: f64,
        secs: f32,
        gain: f32,
        pan: f32,
        sends: Sends,
        mut f: impl FnMut(f32) -> f32,
    ) {
        let len = (secs * SR) as usize;
        let n = self.n;
        let mut idx = ((beat * self.spb).round() as i64).rem_euclid(n as i64) as usize;
        let a = (pan.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
        let (gl, gr) = (a.cos() * gain, a.sin() * gain);
        let fade = 960.min(len / 2).max(1);
        let [rev, dly] = sends;
        for i in 0..len {
            let mut s = f(i as f32 * (1.0 / SR));
            let left = len - i;
            if left < fade {
                s *= left as f32 / fade as f32;
            }
            let (l, r) = (s * gl, s * gr);
            let k = idx * 2;
            self.dry[k] += l;
            self.dry[k + 1] += r;
            if rev > 0.0 {
                self.rev[k] += l * rev;
                self.rev[k + 1] += r * rev;
            }
            if dly > 0.0 {
                self.dly[k] += l * dly;
                self.dly[k + 1] += r * dly;
            }
            idx += 1;
            if idx == n {
                idx = 0;
            }
        }
    }

    /// Slow pad: two slightly detuned additive voices, one per side.
    #[allow(clippy::too_many_arguments)]
    fn pad(
        &mut self,
        beat: f64,
        beats: f64,
        note: i32,
        gain: f32,
        pan: f32,
        att: f32,
        rel: f32,
        bright: f32,
        rev: f32,
    ) {
        let tab = sine();
        // One cycle of the pad's waveform, so the voices need a single lookup per sample.
        let mut wave = Box::new([0.0f32; TAB + 1]);
        for (i, w) in wave.iter_mut().enumerate() {
            let p = i as f32 / TAB as f32;
            *w = fsin(tab, p)
                + bright
                    * (0.42 * fsin(tab, 2.0 * p)
                        + 0.2 * fsin(tab, 3.0 * p)
                        + 0.1 * fsin(tab, 4.0 * p)
                        + 0.05 * fsin(tab, 5.0 * p)
                        + 0.03 * fsin(tab, 6.0 * p));
        }
        let wave = &*wave;
        let f = hz(note);
        let secs = self.secs(beats) + rel;
        for (side, detune) in [(-0.45f32, 0.9976f32), (0.45, 1.0024)] {
            let inc = f * detune / SR;
            let mut p = self.rng.uni();
            self.voice(beat, secs, gain, pan + side, [rev, 0.0], |t| {
                p += inc;
                if p >= 1.0 {
                    p -= 1.0;
                }
                smooth(t / att) * smooth((secs - t) / rel) * fsin(wave, p)
            });
        }
    }

    /// Electric-piano-like key: a sine carrier with a decaying touch of phase modulation.
    fn keys(&mut self, beat: f64, beats: f64, note: i32, vel: f32, pan: f32, sends: Sends) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let f = hz(note);
        let inc = f / SR;
        let rel = 0.2;
        let secs = self.secs(beats) + rel;
        let depth = 1.5 * (330.0 / f).min(1.3) / TAU;
        let floor = 0.4 / TAU;
        let (ka, ki) = (decay(1.6 * (220.0 / f).powf(0.3)), decay(0.22));
        let (mut amp, mut ienv, mut p) = (1.0f32, 1.0f32, 0.0f32);
        self.voice(beat, secs, vel, pan, sends, |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            amp *= ka;
            ienv *= ki;
            let m = fsin(tab, p) * (floor + depth * ienv);
            fsin(tab, p + m) * amp * smooth(t / 0.006) * smooth((secs - t) / rel)
        });
    }

    /// Filtered pluck: soft saw through a two-pole low-pass whose cutoff falls with the note.
    #[allow(clippy::too_many_arguments)]
    fn pluck(
        &mut self,
        beat: f64,
        note: i32,
        vel: f32,
        pan: f32,
        cut: f32,
        sweep: f32,
        tau: f32,
        sends: Sends,
    ) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let inc = hz(note) / SR;
        let (ka, kc) = (decay(tau), decay(tau * 0.35));
        let (mut amp, mut cenv, mut p) = (1.0f32, 1.0f32, 0.0f32);
        let (mut z1, mut z2) = (0.0f32, 0.0f32);
        let w = TAU / SR;
        self.voice(beat, tau * 6.0, vel, pan, sends, |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            amp *= ka;
            cenv *= kc;
            let x = (cut + sweep * cenv) * w;
            let g = x / (1.0 + x);
            let s = 0.7 * saw(p, inc) + 0.3 * fsin(tab, p + 0.5);
            z1 += g * (s - z1);
            z2 += g * (z1 - z2);
            z2 * amp * smooth(t / 0.003)
        });
    }

    /// Marimba-like mallet: fundamental plus quickly dying upper bar partials.
    fn mallet(&mut self, beat: f64, note: i32, vel: f32, pan: f32, sends: Sends) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let f = hz(note);
        let inc = f / SR;
        let tau = (0.42 * (262.0 / f).sqrt()).clamp(0.18, 0.7);
        let (k1, k2, k3) = (decay(tau), decay(tau * 0.18), decay(0.02));
        let (mut a1, mut a2, mut a3) = (1.0f32, 0.36f32, 0.12f32);
        let mut p = 0.0f32;
        self.voice(beat, tau * 6.0, vel, pan, sends, |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            a1 *= k1;
            a2 *= k2;
            a3 *= k3;
            let s = a1 * fsin(tab, p) + a2 * fsin(tab, 4.0 * p) + a3 * fsin(tab, 10.0 * p);
            s * smooth(t / 0.002)
        });
    }

    /// Soft bell: a few harmonics, the upper ones fading first.
    fn chime(&mut self, beat: f64, note: i32, vel: f32, pan: f32, tau: f32, sends: Sends) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let inc = hz(note) / SR;
        let (k1, k2, k3) = (decay(tau), decay(tau * 0.45), decay(tau * 0.25));
        let (mut a1, mut a2, mut a3) = (1.0f32, 0.4f32, 0.2f32);
        let mut p = 0.0f32;
        self.voice(beat, tau * 6.0, vel, pan, sends, |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            a1 *= k1;
            a2 *= k2;
            a3 *= k3;
            let s = a1 * fsin(tab, p)
                + a2 * fsin(tab, 2.0 * p)
                + a3 * (fsin(tab, 3.0 * p) + 0.4 * fsin(tab, 4.0 * p));
            s * smooth(t / 0.005)
        });
    }

    /// Round bass: sine with a little second harmonic at the front of the note.
    fn bass(&mut self, beat: f64, beats: f64, note: i32, vel: f32) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let inc = hz(note) / SR;
        let rel = 0.09;
        let secs = self.secs(beats) + rel;
        let (kb, ks) = (decay(0.12), decay(0.9));
        let (mut body, mut sus, mut p) = (1.0f32, 1.0f32, 0.0f32);
        self.voice(beat, secs, vel, 0.0, [0.0, 0.0], |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            body *= kb;
            sus *= ks;
            let s = fsin(tab, p)
                + (0.2 + 0.4 * body) * fsin(tab, 2.0 * p)
                + (0.08 + 0.15 * body) * fsin(tab, 3.0 * p);
            s * (0.45 + 0.55 * sus) * smooth(t / 0.008) * smooth((secs - t) / rel)
        });
    }

    /// Soft kick: a sine dropping in pitch, no click layer.
    fn kick(&mut self, beat: f64, vel: f32, tau: f32) {
        let tab = sine();
        let vel = vel * (1.0 + 0.06 * self.rng.bip());
        let (ka, kf) = (decay(tau), decay(0.028));
        let (mut amp, mut fenv, mut p) = (1.0f32, 1.0f32, 0.0f32);
        self.voice(beat, tau * 5.5, vel, 0.0, [0.0, 0.0], |t| {
            p += (47.0 + 70.0 * fenv) / SR;
            if p >= 1.0 {
                p -= 1.0;
            }
            amp *= ka;
            fenv *= kf;
            fsin(tab, p) * amp * smooth(t / 0.004)
        });
    }

    /// Band-limited noise burst for hats, shakers and the body of a rim hit.
    #[allow(clippy::too_many_arguments)]
    fn noise(
        &mut self,
        beat: f64,
        vel: f32,
        pan: f32,
        att: f32,
        tau: f32,
        low: f32,
        high: f32,
        rev: f32,
    ) {
        let (beat, vel) = self.human(beat, vel);
        let mut rng = Rng(self.rng.next() as u64 | 1 << 40);
        let (gl, gh) = (pole(low), pole(high));
        let ka = decay(tau);
        let (mut amp, mut h1, mut h2) = (1.0f32, 0.0f32, 0.0f32);
        let mut l = [0.0f32; 4];
        self.voice(beat, att + tau * 6.0, vel, pan, [rev, 0.0], |t| {
            let x = rng.bip();
            h1 += gl * (x - h1);
            let y = x - h1;
            h2 += gl * (y - h2);
            let y = y - h2;
            l[0] += gh * (y - l[0]);
            l[1] += gh * (l[0] - l[1]);
            l[2] += gh * (l[1] - l[2]);
            l[3] += gh * (l[2] - l[3]);
            if t > att {
                amp *= ka;
            }
            l[3] * amp * smooth(t / att)
        });
    }

    fn hat(&mut self, beat: f64, vel: f32, pan: f32) {
        self.noise(beat, vel * 7.0, pan, 0.0015, 0.022, 6000.0, 11_000.0, 0.1);
    }

    fn shaker(&mut self, beat: f64, vel: f32, pan: f32) {
        self.noise(beat, vel * 7.0, pan, 0.012, 0.028, 4500.0, 9000.0, 0.15);
    }

    /// Rim-like backbeat: a short woody tone with a little filtered noise.
    fn rim(&mut self, beat: f64, vel: f32, pan: f32, rev: f32) {
        let tab = sine();
        let (beat, vel) = self.human(beat, vel);
        let ka = decay(0.035);
        let inc = 370.0 / SR;
        let (mut amp, mut p) = (1.0f32, 0.0f32);
        self.voice(beat, 0.22, vel, pan, [rev, 0.0], |t| {
            p += inc;
            if p >= 1.0 {
                p -= 1.0;
            }
            amp *= ka;
            (fsin(tab, p) + 0.35 * fsin(tab, 2.27 * p)) * amp * smooth(t / 0.002)
        });
        self.noise(beat, vel * 1.6, pan, 0.002, 0.045, 900.0, 3500.0, rev);
    }

    /// Runs the sends through delay and reverb, filters, levels and limits the sum.
    fn master(self, fx: &Fx) -> Vec<f32> {
        let Mix {
            n,
            spb,
            mut dry,
            mut rev,
            dly,
            ..
        } = self;

        if fx.dly_level > 0.0 {
            let len = ((fx.dly_beats * spb) as usize).max(1);
            let (mut bl, mut br) = (vec![0.0f32; len], vec![0.0f32; len]);
            let (mut pos, mut fl, mut fr) = (0usize, 0.0f32, 0.0f32);
            let g = pole(2800.0);
            let (fb, lvl) = (fx.dly_fb, fx.dly_level);
            looped(n, len * 16, |i, live| {
                let (ol, or) = (bl[pos], br[pos]);
                fl += g * (or - fl);
                fr += g * (ol - fr);
                bl[pos] = 0.5 * (dly[2 * i] + dly[2 * i + 1]) + fl * fb;
                br[pos] = fr;
                pos += 1;
                if pos == len {
                    pos = 0;
                }
                if live {
                    let (l, r) = (lvl * (0.8 * ol + 0.2 * or), lvl * (0.2 * ol + 0.8 * or));
                    dry[2 * i] += l;
                    dry[2 * i + 1] += r;
                    rev[2 * i] += 0.3 * l;
                    rev[2 * i + 1] += 0.3 * r;
                }
            });
        }

        if fx.rev_level > 0.0 {
            let mut verb = Reverb::new(fx.rev_fb, fx.rev_damp);
            let g = pole(250.0);
            let (mut hl, mut hr) = (0.0f32, 0.0f32);
            let lvl = fx.rev_level;
            // Whole number of wobble cycles per loop, about 0.4 Hz.
            let tab = sine();
            let rate = (0.4 * n as f32 / SR).round().max(1.0) / n as f32;
            // The reverb runs at half rate: its tail has no top end worth the cost.
            let (mut il, mut ir, mut pl, mut pr) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            looped(n, SAMPLE_RATE as usize * 10, |i, live| {
                let (l, r) = (rev[2 * i], rev[2 * i + 1]);
                hl += g * (l - hl);
                hr += g * (r - hr);
                if i % 2 == 0 {
                    il = l - hl;
                    ir = r - hr;
                    return;
                }
                let ph = i as f32 * rate;
                let lfo = (fsin(tab, ph), fsin(tab, ph + 0.25));
                let (wl, wr) = verb.tick(0.5 * (il + l - hl), 0.5 * (ir + r - hr), lfo);
                if live {
                    dry[2 * i - 2] += lvl * 0.5 * (pl + wl);
                    dry[2 * i - 1] += lvl * 0.5 * (pr + wr);
                    dry[2 * i] += lvl * wl;
                    dry[2 * i + 1] += lvl * wr;
                }
                pl = wl;
                pr = wr;
            });
        }

        // Rumble/DC filter, a presence shelf and a gentle top-end roll-off.
        let (gh, gs, gt) = (pole(24.0), pole(1500.0), pole(fx.tone));
        let lift = fx.lift;
        let mut st = [[0.0f32; 5]; 2];
        let mut out = vec![0.0f32; n * 2];
        looped(n, SAMPLE_RATE as usize, |i, live| {
            for ch in 0..2 {
                let s = &mut st[ch];
                let mut x = dry[2 * i + ch];
                s[0] += gh * (x - s[0]);
                x -= s[0];
                s[1] += gh * (x - s[1]);
                x -= s[1];
                s[2] += gs * (x - s[2]);
                x += lift * (x - s[2]);
                s[3] += gt * (x - s[3]);
                s[4] += gt * (s[3] - s[4]);
                if live {
                    out[2 * i + ch] = s[4];
                }
            }
        });

        let level = rms(&out).max(1e-9);
        let mut gain = TARGET_RMS / level;
        for _ in 0..2 {
            let sum: f64 = out.iter().map(|x| (limit(x * gain) as f64).powi(2)).sum();
            let got = (sum / out.len() as f64).sqrt() as f32;
            gain *= TARGET_RMS / got.max(1e-9);
        }
        for x in out.iter_mut() {
            *x = limit(*x * gain);
        }
        out
    }
}

// ---------------------------------------------------------------- tracks

/// Ambient pads in D major, no drums: Dadd9, Bm7, Gmaj7, Asus4, two bars each.
fn open_sky(m: &mut Mix) -> Fx {
    let chords: [([i32; 4], i32); 4] = [
        ([54, 57, 62, 64], 50),
        ([54, 57, 59, 62], 47),
        ([54, 59, 62, 67], 43),
        ([52, 57, 62, 64], 45),
    ];
    let pans = [-0.3, 0.25, -0.1, 0.3];
    for (i, (notes, root)) in chords.iter().enumerate() {
        let b = i as f64 * 8.0;
        for (k, &note) in notes.iter().enumerate() {
            let lead = 1.0 + 0.3 * k as f64;
            m.pad(b - lead, 8.6, note, 0.15, pans[k], 2.6, 3.6, 1.0, 0.3);
        }
        m.pad(b - 0.6, 8.2, *root, 0.13, 0.0, 2.0, 3.0, 0.35, 0.0);
        m.pad(b + 1.0, 6.0, notes[3] + 12, 0.045, 0.0, 3.0, 4.0, 0.15, 0.7);
        m.pad(b + 3.0, 4.0, notes[1] + 12, 0.035, 0.0, 2.5, 3.5, 0.15, 0.7);
    }
    let bells: [(f64, i32); 14] = [
        (1.5, 78),
        (3.0, 76),
        (5.5, 69),
        (9.0, 74),
        (10.5, 71),
        (13.0, 78),
        (16.5, 71),
        (18.0, 74),
        (19.5, 78),
        (22.0, 76),
        (25.0, 76),
        (27.0, 69),
        (29.0, 74),
        (30.5, 76),
    ];
    for (i, &(b, note)) in bells.iter().enumerate() {
        let pan = if i % 2 == 0 { -0.35 } else { 0.35 };
        m.chime(b, note, 0.11, pan, 1.1, [0.7, 0.45]);
    }
    Fx {
        rev_fb: 0.92,
        rev_damp: 0.21,
        rev_level: 0.9,
        dly_beats: 1.5,
        dly_fb: 0.45,
        dly_level: 0.6,
        tone: 11000.0,
        lift: 1.2,
    }
}

/// Lo-fi keys in F major with a swung, soft kit. Bars: A A B A, where
/// A = Fmaj9 Dm9 Gm9 C13 and B = Bbmaj9 Am7 Gm9 C13.
fn slow_coffee(m: &mut Mix) -> Fx {
    const F: ([i32; 4], i32) = ([57, 60, 64, 67], 41);
    const DM: ([i32; 4], i32) = ([57, 60, 64, 65], 38);
    const GM: ([i32; 4], i32) = ([58, 62, 65, 69], 43);
    const C: ([i32; 4], i32) = ([58, 62, 64, 69], 36);
    const BB: ([i32; 4], i32) = ([57, 60, 62, 65], 46);
    const AM: ([i32; 4], i32) = ([57, 60, 64, 67], 45);
    let bars = [F, DM, GM, C, F, DM, GM, C, BB, AM, GM, C, F, DM, GM, C];
    // Delays the off-beat a little.
    let sw = |b: f64| {
        let whole = b.floor();
        let x = b - whole;
        whole
            + if x < 0.5 {
                x * 1.12
            } else {
                0.56 + (x - 0.5) * 0.88
            }
    };
    let pans = [-0.3, 0.1, -0.1, 0.3];
    for (bar, (notes, root)) in bars.iter().enumerate() {
        let b0 = bar as f64 * 4.0;
        let odd = bar % 2 == 1;
        let hits: &[(f64, f64, f32)] = if odd {
            &[(0.0, 1.3, 1.0), (1.5, 1.0, 0.75), (3.0, 0.9, 0.8)]
        } else {
            &[(0.0, 1.8, 1.0), (2.5, 1.3, 0.8)]
        };
        for &(at, len, vel) in hits {
            for (k, &note) in notes.iter().enumerate() {
                let strum = k as f64 * 0.018;
                let v = 0.125 * vel * (1.0 - 0.07 * k as f32);
                m.keys(sw(b0 + at) + strum, len, note, v, pans[k], [0.3, 0.0]);
            }
        }
        m.bass(b0, 1.4, *root, 0.2);
        m.bass(sw(b0 + 2.5), 0.9, *root, 0.17);
        if odd {
            m.bass(sw(b0 + 3.5), 0.4, root + 7, 0.14);
        }
        m.kick(b0, 0.36, 0.12);
        m.kick(sw(b0 + 2.5), 0.3, 0.12);
        m.rim(b0 + 1.0, 0.1, -0.1, 0.25);
        m.rim(b0 + 3.0, 0.1, -0.1, 0.25);
        for e in 0..8 {
            let v = if e % 2 == 0 { 0.09 } else { 0.06 };
            m.hat(sw(b0 + e as f64 * 0.5), v, 0.15);
        }
        if odd {
            m.hat(sw(b0 + 3.75), 0.05, 0.15);
        }
    }
    // (bar, beat, note, length in beats), F major pentatonic
    let tune: [(usize, f64, i32, f64); 33] = [
        (0, 1.5, 69, 0.5),
        (0, 2.0, 72, 1.0),
        (0, 3.0, 74, 0.9),
        (1, 0.5, 72, 0.5),
        (1, 1.0, 69, 1.6),
        (2, 1.5, 67, 0.5),
        (2, 2.0, 69, 0.5),
        (2, 2.5, 74, 1.3),
        (3, 1.0, 72, 0.5),
        (3, 1.5, 69, 0.5),
        (3, 2.0, 67, 1.6),
        (6, 0.5, 77, 0.5),
        (6, 1.0, 74, 0.5),
        (6, 1.5, 72, 1.2),
        (7, 2.0, 69, 0.5),
        (7, 2.5, 72, 1.2),
        (8, 1.0, 74, 1.2),
        (8, 2.5, 77, 1.2),
        (9, 0.5, 76, 0.5),
        (9, 1.0, 72, 1.6),
        (10, 1.0, 74, 0.5),
        (10, 1.5, 70, 0.5),
        (10, 2.0, 69, 1.2),
        (11, 1.5, 67, 0.5),
        (11, 2.0, 72, 1.6),
        (12, 1.5, 69, 0.5),
        (12, 2.0, 72, 1.0),
        (12, 3.0, 74, 0.9),
        (13, 0.5, 72, 0.5),
        (13, 1.0, 69, 1.6),
        (14, 1.5, 67, 0.5),
        (14, 2.5, 74, 1.3),
        (15, 2.0, 67, 1.6),
    ];
    for &(bar, at, note, len) in tune.iter() {
        m.keys(
            sw(bar as f64 * 4.0 + at),
            len,
            note,
            0.15,
            -0.15,
            [0.4, 0.5],
        );
    }
    Fx {
        rev_fb: 0.84,
        rev_damp: 0.27,
        rev_level: 0.7,
        dly_beats: 0.75,
        dly_fb: 0.3,
        dly_level: 0.45,
        tone: 10000.0,
        lift: 1.0,
    }
}

/// Upbeat pluck arpeggio in C major over a light four-on-the-floor.
/// Bars: C G Am F, C G Am F, Am F C G, C G Am F.
fn bright_side(m: &mut Mix) -> Fx {
    // (arp tones, pad triad, bass root)
    type Chord = ([i32; 5], [i32; 3], i32);
    const C: Chord = ([60, 64, 67, 72, 74], [55, 60, 64], 36);
    const G: Chord = ([59, 62, 67, 71, 74], [55, 59, 62], 43);
    const AM: Chord = ([60, 64, 69, 72, 76], [57, 60, 64], 45);
    const F: Chord = ([60, 65, 69, 72, 77], [57, 60, 65], 41);
    let bars = [C, G, AM, F, C, G, AM, F, AM, F, C, G, C, G, AM, F];
    let shapes: [[usize; 8]; 2] = [[0, 2, 3, 1, 2, 4, 3, 2], [0, 1, 2, 3, 4, 3, 2, 1]];
    for (bar, (tones, triad, root)) in bars.iter().enumerate() {
        let b0 = bar as f64 * 4.0;
        let middle = (8..12).contains(&bar);
        let shape = &shapes[middle as usize];
        for (step, &ix) in shape.iter().enumerate() {
            let accent = if step % 4 == 0 { 1.0 } else { 0.8 };
            let pan = if step % 2 == 0 { -0.3 } else { 0.3 };
            m.pluck(
                b0 + step as f64 * 0.5,
                tones[ix],
                0.17 * accent,
                pan,
                800.0,
                3000.0,
                0.16,
                [0.25, 0.4],
            );
        }
        for (k, &note) in triad.iter().enumerate() {
            m.pad(
                b0 - 0.2,
                3.7,
                note,
                0.05,
                [-0.25, 0.0, 0.25][k],
                0.35,
                0.7,
                0.5,
                0.4,
            );
        }
        for &(at, len, vel) in &[
            (0.0, 0.9, 0.19f32),
            (1.5, 0.4, 0.15),
            (2.0, 0.9, 0.17),
            (3.5, 0.4, 0.15),
        ] {
            m.bass(b0 + at, len, *root, vel);
        }
        let thin = bar == 8 || bar == 9;
        for beat in 0..4 {
            let b = b0 + beat as f64;
            if !thin {
                m.kick(b, 0.3, 0.1);
            }
            m.hat(b + 0.5, 0.11, 0.2);
            if beat % 2 == 1 {
                m.rim(b, 0.08, -0.1, 0.35);
            }
            for s in 0..4 {
                m.shaker(b + s as f64 * 0.25, if s == 2 { 0.05 } else { 0.03 }, -0.3);
            }
        }
    }
    // (bar, beat, note), C major pentatonic
    let tune: [(usize, f64, i32); 30] = [
        (0, 0.0, 79),
        (0, 1.5, 76),
        (1, 0.0, 74),
        (1, 3.0, 79),
        (2, 0.0, 81),
        (2, 1.5, 79),
        (2, 2.0, 76),
        (3, 0.0, 72),
        (3, 1.5, 74),
        (3, 2.0, 72),
        (3, 3.0, 69),
        (6, 0.0, 76),
        (6, 1.5, 79),
        (6, 2.0, 81),
        (7, 0.0, 77),
        (7, 1.5, 76),
        (7, 2.0, 72),
        (8, 0.0, 76),
        (9, 0.0, 77),
        (9, 1.5, 81),
        (10, 0.0, 79),
        (11, 0.0, 74),
        (11, 1.5, 79),
        (11, 2.0, 74),
        (12, 0.0, 79),
        (12, 1.5, 76),
        (13, 0.0, 74),
        (14, 0.0, 81),
        (14, 2.0, 76),
        (15, 0.0, 72),
    ];
    for &(bar, at, note) in tune.iter() {
        m.chime(bar as f64 * 4.0 + at, note, 0.085, 0.1, 0.45, [0.5, 0.35]);
    }
    Fx {
        rev_fb: 0.82,
        rev_damp: 0.24,
        rev_level: 0.6,
        dly_beats: 0.75,
        dly_fb: 0.35,
        dly_level: 0.5,
        tone: 13000.0,
        lift: 1.2,
    }
}

/// Mallet ostinato in G major over a soft bass, two bars per chord:
/// G Em7 Cadd9 D, G Bm7 Cadd9 Dsus4.
fn wooden_steps(m: &mut Mix) -> Fx {
    let chords: [([i32; 5], i32); 8] = [
        ([55, 62, 67, 71, 74], 43),
        ([55, 59, 64, 67, 71], 40),
        ([55, 60, 64, 67, 74], 48),
        ([57, 62, 66, 69, 74], 50),
        ([55, 62, 67, 71, 74], 43),
        ([57, 62, 66, 71, 74], 47),
        ([55, 60, 64, 67, 74], 48),
        ([57, 62, 67, 69, 74], 50),
    ];
    let shapes: [[usize; 8]; 2] = [[0, 2, 3, 1, 2, 4, 2, 3], [0, 2, 4, 1, 3, 2, 4, 3]];
    for (i, (tones, root)) in chords.iter().enumerate() {
        let b0 = i as f64 * 8.0;
        for (bar, shape) in shapes.iter().enumerate() {
            for (step, &ix) in shape.iter().enumerate() {
                let b = b0 + bar as f64 * 4.0 + step as f64 * 0.5;
                let accent = if matches!(step, 0 | 3 | 6) { 1.0 } else { 0.62 };
                let pan = if step % 2 == 0 { -0.35 } else { 0.35 };
                m.mallet(b, tones[ix], 0.2 * accent, pan, [0.3, 0.25]);
            }
        }
        m.bass(b0, 3.0, *root, 0.17);
        m.bass(b0 + 4.0, 2.2, *root, 0.15);
        m.bass(b0 + 6.5, 1.2, root + 7, 0.12);
        for e in 0..16 {
            let v = if e % 4 == 2 { 0.05 } else { 0.028 };
            m.shaker(b0 + e as f64 * 0.5, v, 0.3);
        }
    }
    // (beat, note), G major pentatonic
    let tune: [(f64, i32); 20] = [
        (0.0, 74),
        (4.0, 76),
        (6.0, 74),
        (8.0, 71),
        (12.0, 79),
        (16.0, 76),
        (19.0, 74),
        (20.0, 76),
        (24.0, 69),
        (28.0, 74),
        (30.0, 76),
        (32.0, 79),
        (36.0, 76),
        (38.0, 74),
        (40.0, 71),
        (42.0, 74),
        (44.0, 81),
        (48.0, 79),
        (52.0, 76),
        (56.0, 74),
    ];
    for &(b, note) in tune.iter() {
        m.chime(b, note, 0.07, -0.1, 0.8, [0.6, 0.3]);
    }
    Fx {
        rev_fb: 0.86,
        rev_damp: 0.24,
        rev_level: 0.7,
        dly_beats: 0.75,
        dly_fb: 0.3,
        dly_level: 0.5,
        tone: 12000.0,
        lift: 1.0,
    }
}

/// Muted pulses in A minor, four bars per chord: Am9, Fmaj7, Cmaj7, Em7.
fn deep_focus(m: &mut Mix) -> Fx {
    // (pad voicing, bass root, upper pulse notes)
    let chords: [([i32; 4], i32, [i32; 2]); 4] = [
        ([57, 60, 64, 71], 45, [69, 72]),
        ([57, 60, 64, 65], 41, [69, 72]),
        ([55, 60, 64, 71], 48, [67, 71]),
        ([55, 59, 62, 64], 40, [67, 71]),
    ];
    let pans = [-0.3, 0.2, -0.1, 0.3];
    for (i, (notes, root, upper)) in chords.iter().enumerate() {
        let b0 = i as f64 * 16.0;
        for (k, &note) in notes.iter().enumerate() {
            m.pad(b0 - 0.8, 16.4, note, 0.07, pans[k], 1.6, 2.4, 0.5, 0.3);
        }
        for bar in 0..4 {
            let bb = b0 + bar as f64 * 4.0;
            for step in 0..8 {
                let b = bb + step as f64 * 0.5;
                // The filter opens and closes once per loop, with a smaller four-bar ripple.
                let sweep =
                    (TAU * (b / 64.0) as f32).sin() * 0.5 + (TAU * (b / 16.0) as f32).sin() * 0.2;
                let cut = 800.0 + 400.0 * (1.0 + sweep);
                let accent = if step % 2 == 0 { 1.0 } else { 0.72 };
                m.pluck(b, 64, 0.17 * accent, -0.25, cut, 500.0, 0.11, [0.2, 0.3]);
                if matches!(step, 0 | 3 | 6) {
                    m.pluck(
                        b,
                        upper[(step / 3) % 2],
                        0.12,
                        0.35,
                        cut + 150.0,
                        600.0,
                        0.13,
                        [0.3, 0.45],
                    );
                }
            }
            for beat in 0..4 {
                let b = bb + beat as f64;
                m.bass(b, 0.55, *root, if beat == 0 { 0.17 } else { 0.13 });
                if beat % 2 == 0 {
                    m.kick(b, 0.2, 0.09);
                }
                m.hat(b + 0.5, 0.07, 0.15);
            }
        }
        let high = [[71, 76], [69, 72], [74, 79], [71, 74]][i];
        m.chime(b0 + 6.0, high[0], 0.05, 0.2, 1.0, [0.7, 0.4]);
        m.chime(b0 + 12.0, high[1], 0.05, -0.2, 1.0, [0.7, 0.4]);
    }
    Fx {
        rev_fb: 0.88,
        rev_damp: 0.27,
        rev_level: 0.7,
        dly_beats: 0.75,
        dly_fb: 0.4,
        dly_level: 0.55,
        tone: 10000.0,
        lift: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all() -> &'static Vec<(&'static TrackInfo, Vec<f32>)> {
        static ALL: OnceLock<Vec<(&'static TrackInfo, Vec<f32>)>> = OnceLock::new();
        ALL.get_or_init(|| {
            tracks()
                .iter()
                .map(|t| (t, render(t.id).expect("listed track renders")))
                .collect()
        })
    }

    fn db(x: f32) -> f32 {
        20.0 * x.log10()
    }

    #[test]
    fn five_tracks_with_stable_ids() {
        let ids: Vec<_> = tracks().iter().map(|t| t.id).collect();
        assert_eq!(
            ids,
            [
                "open-sky",
                "slow-coffee",
                "bright-side",
                "wooden-steps",
                "deep-focus"
            ]
        );
        for t in tracks() {
            assert!(t.id.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
            assert!((24_000..=48_000).contains(&t.duration_ms), "{}", t.id);
        }
    }

    #[test]
    fn unknown_id_is_none() {
        assert!(render("nope").is_none());
        assert!(render("").is_none());
        assert!(render("builtin:open-sky").is_none());
    }

    #[test]
    fn loops_are_whole_bars_of_the_right_length() {
        for (t, s) in all() {
            assert_eq!(s.len(), t.duration_ms as usize * 48 * 2, "{}", t.id);
            let beats = t.duration_ms as f64 * t.bpm as f64 / 60_000.0;
            assert_eq!(beats.fract(), 0.0, "{}", t.id);
            assert_eq!(beats as u32 % 4, 0, "{}", t.id);
        }
    }

    #[test]
    fn deterministic() {
        let (t, first) = &all()[1];
        assert_eq!(&render(t.id).unwrap(), first);
    }

    #[test]
    fn finite_and_below_peak_limit() {
        for (t, s) in all() {
            assert!(s.iter().all(|x| x.is_finite()), "{}", t.id);
            let peak = s.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(peak < 0.9, "{} peaks at {peak}", t.id);
            assert!(peak > 0.2, "{} peaks at {peak}", t.id);
        }
    }

    #[test]
    fn levels_match_across_tracks() {
        for (t, s) in all() {
            let level = db(rms(s));
            assert!((level + 20.0).abs() < 0.5, "{} is at {level} dB", t.id);
        }
    }

    #[test]
    fn no_dc_offset() {
        for (t, s) in all() {
            for ch in 0..2 {
                let mean = s.iter().skip(ch).step_by(2).map(|x| *x as f64).sum::<f64>()
                    / (s.len() / 2) as f64;
                assert!(mean.abs() < 1e-4, "{} channel {ch} mean {mean}", t.id);
            }
        }
    }

    #[test]
    fn no_long_silence() {
        for (t, s) in all() {
            for (i, block) in s.chunks(48_000).enumerate() {
                assert!(
                    db(rms(block)) > -45.0,
                    "{} is silent around {:.1}s",
                    t.id,
                    i as f32 * 0.5
                );
            }
        }
    }

    #[test]
    fn loop_seam_is_continuous() {
        for (t, s) in all() {
            let frames = s.len() / 2;
            for ch in 0..2 {
                let at = |i: usize| s[(i % frames) * 2 + ch];
                let mut deltas: Vec<f32> =
                    (0..frames - 1).map(|i| (at(i + 1) - at(i)).abs()).collect();
                let mean_sq =
                    deltas.iter().map(|d| (*d as f64).powi(2)).sum::<f64>() / deltas.len() as f64;
                let typical = mean_sq.sqrt() as f32;
                deltas.sort_by(|a, b| a.total_cmp(b));
                let p99 = deltas[deltas.len() * 99 / 100];
                // The step across the join, and the steps just either side of it.
                for i in frames - 4..frames + 3 {
                    let step = (at(i + 1) - at(i)).abs();
                    assert!(
                        step <= p99,
                        "{} ch{ch}: step {step} at the seam, p99 {p99}",
                        t.id
                    );
                }
                let seam = (at(0) - at(frames - 1)).abs();
                assert!(
                    seam <= 3.0 * typical,
                    "{} ch{ch}: seam {seam}, typical {typical}",
                    t.id
                );
                // No kink either: the slope carries on across the join.
                let bend = (at(frames + 1) - 2.0 * at(frames) + at(frames - 1)).abs();
                assert!(bend <= p99, "{} ch{ch}: bend {bend}", t.id);
            }
        }
    }

    #[test]
    fn stereo_but_mono_compatible() {
        for (t, s) in all() {
            let (mut ll, mut rr, mut lr, mut diff) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
            for i in (0..s.len()).step_by(2) {
                let (l, r) = (s[i] as f64, s[i + 1] as f64);
                ll += l * l;
                rr += r * r;
                lr += l * r;
                diff += (l - r) * (l - r);
            }
            let corr = lr / (ll * rr).sqrt();
            assert!(
                diff > 0.0 && corr < 0.98,
                "{} is nearly mono ({corr})",
                t.id
            );
            assert!(corr > 0.3, "{} channels barely correlate ({corr})", t.id);
            let balance = db((ll / rr).sqrt() as f32);
            assert!(
                balance.abs() < 1.5,
                "{} leans {balance} dB to one side",
                t.id
            );
        }
    }

    #[test]
    fn info_serialises_in_camel_case() {
        fn check<T: serde::Serialize>(_: &T) {}
        check(&tracks()[0]);
    }
}
