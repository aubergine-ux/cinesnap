//! Text for the export compositor, rasterised from the bundled fonts.
//!
//! Layout is deliberately plain so the canvas preview can reproduce it
//! exactly: glyphs are placed by their advance widths alone (no kerning, no
//! ligatures) and vertical positions never depend on font metrics.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use fontdue::{Font, FontSettings};

/// Font families that can be chosen in a project. The first is the fallback.
pub const FAMILIES: [&str; 4] = ["Inter", "Poppins", "PT Serif", "IBM Plex Mono"];

/// Distance between baselines, as a multiple of the font size.
pub const LINE_HEIGHT: f32 = 1.25;
/// Baseline of the first line below the top of a text block, per font size.
pub const FIRST_BASELINE: f32 = 0.95;
/// Baseline below the vertical centre of a single centred line, per font size.
pub const CENTER_BASELINE: f32 = 0.35;

macro_rules! font {
    ($file:literal) => {
        include_bytes!(concat!("../../../assets/fonts/", $file)) as &[u8]
    };
}

struct Registry {
    /// Regular and bold face per family, in `FAMILIES` order.
    faces: Vec<[Font; 2]>,
    glyphs: Mutex<HashMap<(usize, char, u32), Glyph>>,
}

#[derive(Clone)]
struct Glyph {
    xmin: i32,
    ymin: i32,
    width: usize,
    height: usize,
    coverage: std::sync::Arc<Vec<u8>>,
}

fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let load = |bytes: &[u8]| {
            Font::from_bytes(bytes, FontSettings::default()).expect("bundled font is valid")
        };
        Registry {
            faces: vec![
                [
                    load(font!("Inter-Regular.otf")),
                    load(font!("Inter-Bold.otf")),
                ],
                [
                    load(font!("Poppins-Regular.ttf")),
                    load(font!("Poppins-Bold.ttf")),
                ],
                [
                    load(font!("PTSerif-Regular.ttf")),
                    load(font!("PTSerif-Bold.ttf")),
                ],
                [
                    load(font!("IBMPlexMono-Regular.ttf")),
                    load(font!("IBMPlexMono-Bold.ttf")),
                ],
            ],
            glyphs: Mutex::new(HashMap::new()),
        }
    })
}

#[derive(Clone, Copy, Debug)]
pub struct TextStyle<'a> {
    pub family: &'a str,
    pub bold: bool,
    /// Font size (em height) in pixels.
    pub px: f32,
}

impl TextStyle<'_> {
    fn face(&self) -> usize {
        let family = FAMILIES
            .iter()
            .position(|f| f.eq_ignore_ascii_case(self.family))
            .unwrap_or(0);
        family * 2 + self.bold as usize
    }

    fn font(&self) -> &'static Font {
        let face = self.face();
        &registry().faces[face / 2][face % 2]
    }
}

/// Width of a single line of text.
pub fn measure(style: TextStyle, text: &str) -> f32 {
    let font = style.font();
    text.chars()
        .map(|c| font.metrics(c, style.px).advance_width)
        .sum()
}

/// Draw one line of text with its baseline starting at `(x, baseline)`.
/// `mask`, when given, is one coverage byte per pixel of the target.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    buf: &mut [u8],
    width: usize,
    height: usize,
    mask: Option<&[u8]>,
    style: TextStyle,
    text: &str,
    x: f32,
    baseline: f32,
    color: [u8; 4],
    alpha: f64,
) {
    let alpha = (color[3] as f64 * alpha.clamp(0.0, 1.0)) as u32;
    if alpha == 0 || style.px < 1.0 || !style.px.is_finite() {
        return;
    }
    let reg = registry();
    let font = style.font();
    let face = style.face();
    let size_key = (style.px * 4.0).round() as u32;
    let px = size_key as f32 / 4.0;
    let mut pen = x;
    for ch in text.chars() {
        let glyph = {
            let mut cache = reg.glyphs.lock().unwrap();
            if cache.len() > 6000 {
                cache.clear();
            }
            cache
                .entry((face, ch, size_key))
                .or_insert_with(|| {
                    let (m, coverage) = font.rasterize(ch, px);
                    Glyph {
                        xmin: m.xmin,
                        ymin: m.ymin,
                        width: m.width,
                        height: m.height,
                        coverage: std::sync::Arc::new(coverage),
                    }
                })
                .clone()
        };
        let gx = pen.round() as i64 + glyph.xmin as i64;
        let gy = baseline.round() as i64 - glyph.ymin as i64 - glyph.height as i64;
        for row in 0..glyph.height {
            let y = gy + row as i64;
            if y < 0 || y >= height as i64 {
                continue;
            }
            for col in 0..glyph.width {
                let x = gx + col as i64;
                if x < 0 || x >= width as i64 {
                    continue;
                }
                let i = y as usize * width + x as usize;
                let mut a = glyph.coverage[row * glyph.width + col] as u32 * alpha / 255;
                if let Some(m) = mask {
                    a = a * m[i] as u32 / 255;
                }
                if a == 0 {
                    continue;
                }
                let p = &mut buf[i * 4..i * 4 + 3];
                for ch in 0..3 {
                    p[ch] = ((p[ch] as u32 * (255 - a) + color[ch] as u32 * a + 127) / 255) as u8;
                }
            }
        }
        // Advance by the unrounded size so layout matches `measure`.
        pen += font.metrics(ch, style.px).advance_width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_family_measures_and_draws() {
        for family in FAMILIES {
            for bold in [false, true] {
                let style = TextStyle {
                    family,
                    bold,
                    px: 40.0,
                };
                let w = measure(style, "Hello");
                assert!(w > 60.0 && w < 160.0, "{family} {w}");
                let mut buf = vec![0u8; 200 * 60 * 4];
                draw(
                    &mut buf,
                    200,
                    60,
                    None,
                    style,
                    "Hello",
                    4.0,
                    45.0,
                    [255, 255, 255, 255],
                    1.0,
                );
                let lit = buf.chunks(4).filter(|p| p[0] > 128).count();
                assert!(lit > 150, "{family} drew {lit} px");
                // Nothing below the baseline for text without descenders.
                assert!(buf[48 * 200 * 4..].iter().all(|&b| b == 0));
            }
        }
        let regular = TextStyle {
            family: "Inter",
            bold: false,
            px: 40.0,
        };
        let bold = TextStyle {
            bold: true,
            ..regular
        };
        assert!(measure(bold, "Hello") > measure(regular, "Hello"));
        // Unknown families fall back instead of failing.
        let fallback = TextStyle {
            family: "Comic Sans",
            ..regular
        };
        assert_eq!(measure(fallback, "abc"), measure(regular, "abc"));
        // Width scales with size.
        let big = TextStyle {
            px: 80.0,
            ..regular
        };
        assert!((measure(big, "abc") - 2.0 * measure(regular, "abc")).abs() < 0.01);
    }

    #[test]
    fn mask_and_bounds_are_respected() {
        let style = TextStyle {
            family: "Inter",
            bold: true,
            px: 30.0,
        };
        let mut buf = vec![0u8; 50 * 40 * 4];
        let mask = vec![0u8; 50 * 40];
        draw(
            &mut buf,
            50,
            40,
            Some(&mask),
            style,
            "MMMM",
            -20.0,
            30.0,
            [255, 0, 0, 255],
            1.0,
        );
        assert!(buf.iter().all(|&b| b == 0));
        // Far outside the buffer: must not panic.
        draw(
            &mut buf,
            50,
            40,
            None,
            style,
            "MMMM",
            1e6,
            -1e6,
            [255, 0, 0, 255],
            1.0,
        );
    }
}
