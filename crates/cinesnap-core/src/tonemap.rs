//! HDR to SDR for screen recordings.
//!
//! An HDR desktop is captured as PQ-encoded BT.2020. Nearly everything on it
//! is SDR content (windows, text, the UI) that the compositor placed at the
//! user's SDR white level, so the conversion reproduces that content as an
//! SDR capture would and clips what is brighter. It runs as three cheap
//! ffmpeg stages on 16-bit RGB: a 1D LUT from PQ to linear light, the
//! BT.2020 to BT.709 matrix, and a 1D LUT from linear light to sRGB.

use std::io::Write;
use std::path::Path;

use anyhow::Result;

/// Reference white of BT.2408, for when the desktop's own is unknown.
pub const REFERENCE_NITS: f64 = 203.0;
pub const LUT_SIZE: usize = 4096;

/// BT.2020 to BT.709 primaries, linear light, row by row.
pub const BT2020_TO_BT709: [[f64; 3]; 3] = [
    [1.660_491, -0.587_641, -0.072_850],
    [-0.124_550, 1.132_900, -0.008_349],
    [-0.018_151, -0.100_579, 1.118_730],
];

/// PQ (SMPTE ST 2084) signal to absolute luminance in nits.
pub fn pq_to_nits(e: f64) -> f64 {
    const M1: f64 = 2610.0 / 16384.0;
    const M2: f64 = 2523.0 / 4096.0 * 128.0;
    const C1: f64 = 3424.0 / 4096.0;
    const C2: f64 = 2413.0 / 4096.0 * 32.0;
    const C3: f64 = 2392.0 / 4096.0 * 32.0;
    let p = e.clamp(0.0, 1.0).powf(1.0 / M2);
    ((p - C1).max(0.0) / (C2 - C3 * p)).powf(1.0 / M1) * 10_000.0
}

/// Linear light to the sRGB signal.
pub fn srgb_encode(l: f64) -> f64 {
    let l = l.clamp(0.0, 1.0);
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

fn write_1d(path: &Path, f: impl Fn(f64) -> f64) -> Result<()> {
    let tmp = path.with_extension("part");
    let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    writeln!(out, "LUT_1D_SIZE {LUT_SIZE}")?;
    for i in 0..LUT_SIZE {
        let v = f(i as f64 / (LUT_SIZE - 1) as f64);
        writeln!(out, "{v:.7} {v:.7} {v:.7}")?;
    }
    out.flush()?;
    drop(out);
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// The ffmpeg filters that turn 16-bit PQ BT.2020 RGB from a desktop with
/// SDR white at `white` nits into sRGB. Their LUT files are written into
/// `dir` on first use and named relative to it, so ffmpeg must run there.
pub fn filters(dir: &Path, white: f64) -> Result<Vec<String>> {
    let white = white.clamp(40.0, 1000.0).round();
    let linear = format!("cinesnap-pq-linear-{white}.cube");
    let srgb = "cinesnap-linear-srgb.cube";
    std::fs::create_dir_all(dir)?;
    if !dir.join(&linear).is_file() {
        write_1d(&dir.join(&linear), |e| (pq_to_nits(e) / white).min(1.0))?;
    }
    if !dir.join(srgb).is_file() {
        write_1d(&dir.join(srgb), srgb_encode)?;
    }
    let names = ["r", "g", "b"];
    let mut mix = Vec::new();
    for (i, row) in BT2020_TO_BT709.iter().enumerate() {
        for (j, v) in row.iter().enumerate() {
            mix.push(format!("{}{}={v}", names[i], names[j]));
        }
    }
    Ok(vec![
        "format=rgb48le".into(),
        format!("lut1d=file={linear}:interp=linear"),
        format!("colorchannelmixer={}", mix.join(":")),
        format!("lut1d=file={srgb}:interp=linear"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Absolute luminance in nits to a PQ signal.
    fn nits_to_pq(y: f64) -> f64 {
        const M1: f64 = 2610.0 / 16384.0;
        const M2: f64 = 2523.0 / 4096.0 * 128.0;
        const C1: f64 = 3424.0 / 4096.0;
        const C2: f64 = 2413.0 / 4096.0 * 32.0;
        const C3: f64 = 2392.0 / 4096.0 * 32.0;
        let p = (y / 10_000.0).powf(M1);
        ((C1 + C2 * p) / (1.0 + C3 * p)).powf(M2)
    }

    fn srgb_decode(c: f64) -> f64 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    /// Read a 1D `.cube` back and apply it the way `lut1d` does.
    fn lut(path: &Path) -> impl Fn(f64) -> f64 {
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines = text.lines();
        assert_eq!(
            lines.next(),
            Some(format!("LUT_1D_SIZE {LUT_SIZE}").as_str())
        );
        let v: Vec<f64> = lines
            .map(|l| l.split(' ').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(v.len(), LUT_SIZE);
        move |x: f64| {
            let f = x.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f64;
            let i = (f.floor() as usize).min(LUT_SIZE - 2);
            v[i] + (v[i + 1] - v[i]) * (f - i as f64)
        }
    }

    /// The filter chain, simulated in 16 bits per channel.
    fn chain(dir: &Path, white: f64) -> impl Fn([f64; 3]) -> [f64; 3] {
        let names = filters(dir, white).unwrap();
        let file = |i: usize| {
            let f = names[i].split("file=").nth(1).unwrap();
            dir.join(f.split(':').next().unwrap())
        };
        let (to_linear, to_srgb) = (lut(&file(1)), lut(&file(3)));
        let q = |v: f64| (v.clamp(0.0, 1.0) * 65535.0).round() / 65535.0;
        move |pq: [f64; 3]| {
            let l = pq.map(|v| q(to_linear(q(v))));
            let m = BT2020_TO_BT709;
            let c: [f64; 3] =
                std::array::from_fn(|i| q(m[i][0] * l[0] + m[i][1] * l[1] + m[i][2] * l[2]));
            c.map(|v| q(to_srgb(v)))
        }
    }

    /// What a compositor puts on an HDR desktop for an sRGB colour.
    fn compose(srgb: [f64; 3], white: f64) -> [f64; 3] {
        let l = srgb.map(srgb_decode);
        let m = [
            [0.627_404, 0.329_283, 0.043_313],
            [0.069_097, 0.919_540, 0.011_362],
            [0.016_391, 0.088_013, 0.895_595],
        ];
        std::array::from_fn(|i| {
            nits_to_pq((m[i][0] * l[0] + m[i][1] * l[1] + m[i][2] * l[2]) * white)
        })
    }

    #[test]
    fn pq_matches_the_standard() {
        assert!(pq_to_nits(0.0).abs() < 1e-9);
        assert!((pq_to_nits(1.0) - 10_000.0).abs() < 1e-6);
        // 100 nits is a PQ signal of about 0.5081.
        assert!((pq_to_nits(0.508_1) - 100.0).abs() < 0.2);
        for nits in [0.5, 10.0, 203.0, 1000.0] {
            assert!((pq_to_nits(nits_to_pq(nits)) - nits).abs() < nits * 1e-6);
        }
    }

    #[test]
    fn sdr_colours_come_back_as_they_were() {
        let dir = tempfile::tempdir().unwrap();
        for white in [REFERENCE_NITS, 120.0, 400.0] {
            let run = chain(dir.path(), white);
            let mut worst: f64 = 0.0;
            for i in 0..5000 {
                // A fixed spread of colours over the whole sRGB cube.
                let h = |k: u32| (i as f64 * (0.618_034 + k as f64 * 0.414_214)).fract();
                let srgb = [h(0), h(1), h(2)];
                let out = run(compose(srgb, white));
                for k in 0..3 {
                    worst = worst.max((srgb[k] - out[k]).abs());
                }
            }
            // Within one 8-bit step.
            assert!(worst < 1.0 / 255.0, "white {white}: worst error {worst}");
        }
    }

    #[test]
    fn highlights_clip_to_white() {
        let dir = tempfile::tempdir().unwrap();
        let run = chain(dir.path(), REFERENCE_NITS);
        let white = run([nits_to_pq(REFERENCE_NITS); 3]);
        let brighter = run([nits_to_pq(1000.0); 3]);
        assert!(white.iter().all(|&c| c > 0.999), "{white:?}");
        assert_eq!(brighter, [1.0; 3]);
        let mut last = 0.0;
        for nits in [0.1, 5.0, 50.0, 150.0, 203.0, 2000.0] {
            let v = run([nits_to_pq(nits); 3])[1];
            assert!(v >= last, "{nits}: {v}");
            last = v;
        }
    }

    #[test]
    fn filters_reuse_their_luts() {
        let dir = tempfile::tempdir().unwrap();
        let a = filters(dir.path(), 240.4).unwrap();
        let b = filters(dir.path(), 240.0).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[0], "format=rgb48le");
        assert!(a[2].starts_with("colorchannelmixer=rr=1.660491:rg=-0.587641"));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
