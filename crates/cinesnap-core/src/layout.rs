//! Output geometry shared by the preview and the exporter.

use crate::project::{Project, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// Output size in pixels (always even).
    pub width: u32,
    pub height: u32,
    /// Scale of all "pixel" style values relative to a 1080p output.
    pub s: f64,
    /// Where the recording sits inside the output.
    pub frame: Rect,
    /// Corner radius of the frame in output pixels.
    pub radius: f64,
}

fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32 * 2).max(2)
}

/// Width / height for an aspect preset; `source` (or anything unknown) uses
/// the crop's own aspect.
pub fn aspect_ratio(aspect: &str, crop: Rect) -> f64 {
    match aspect.split_once(':') {
        Some((a, b)) => match (a.parse::<f64>(), b.parse::<f64>()) {
            (Ok(a), Ok(b)) if a > 0.0 && b > 0.0 => a / b,
            _ => crop.width / crop.height,
        },
        None => crop.width / crop.height,
    }
}

/// Output size for a preset. Video presets fix the shorter side, GIF presets
/// fix the width.
pub fn output_size(preset: &str, ratio: f64) -> (u32, u32) {
    let by_width = |w: f64| (even(w), even(w / ratio));
    let by_short = |p: f64| {
        if ratio >= 1.0 {
            (even(p * ratio), even(p))
        } else {
            (even(p), even(p / ratio))
        }
    };
    match preset {
        "4k" => by_short(2160.0),
        "1440p" => by_short(1440.0),
        "720p" => by_short(720.0),
        "readme" => by_width(800.0),
        "small" => by_width(480.0),
        _ => by_short(1080.0),
    }
}

/// Output frame rate: GIF presets carry their own, video uses the setting.
pub fn output_fps(project: &Project) -> u32 {
    if project.export.format == "gif" {
        match project.export.preset.as_str() {
            "readme" => return 15,
            "small" => return 12,
            _ => {}
        }
    }
    project.export.fps.max(1)
}

impl Layout {
    pub fn new(project: &Project) -> Self {
        let ratio = aspect_ratio(&project.frame.aspect, project.crop);
        let (width, height) = output_size(&project.export.preset, ratio);
        Self::with_size(project, width, height)
    }

    pub fn with_size(project: &Project, width: u32, height: u32) -> Self {
        let (w, h) = (width as f64, height as f64);
        let short = w.min(h);
        let s = short / 1080.0;
        let pad = project.frame.padding * short;
        let (aw, ah) = ((w - 2.0 * pad).max(2.0), (h - 2.0 * pad).max(2.0));
        let crop_ratio = project.crop.width / project.crop.height;
        let (fw, fh) = if aw / ah > crop_ratio {
            (ah * crop_ratio, ah)
        } else {
            (aw, aw / crop_ratio)
        };
        Self {
            width,
            height,
            s,
            frame: Rect {
                x: (w - fw) / 2.0,
                y: (h - fh) / 2.0,
                width: fw,
                height: fh,
            },
            radius: (project.frame.radius * s).min(fw.min(fh) / 2.0),
        }
    }
}

/// The part of the source visible for a camera state.
pub fn view_rect(crop: Rect, cx: f64, cy: f64, scale: f64) -> Rect {
    let (w, h) = (crop.width / scale, crop.height / scale);
    Rect {
        x: cx - w / 2.0,
        y: cy - h / 2.0,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::project;

    #[test]
    fn presets() {
        assert_eq!(output_size("1080p", 16.0 / 9.0), (1920, 1080));
        assert_eq!(output_size("1080p", 9.0 / 16.0), (1080, 1920));
        assert_eq!(output_size("4k", 1.0), (2160, 2160));
        assert_eq!(output_size("readme", 16.0 / 9.0), (800, 450));
        assert_eq!(output_size("720p", 1366.0 / 768.0), (1280, 720));
    }

    #[test]
    fn frame_fits_and_keeps_aspect() {
        let mut p = project(2560, 1440, 1000.0);
        let l = Layout::new(&p);
        assert_eq!((l.width, l.height), (1920, 1080));
        let pad = 0.06 * 1080.0;
        assert!((l.frame.height - (1080.0 - 2.0 * pad)).abs() < 1e-9);
        assert!((l.frame.width / l.frame.height - 16.0 / 9.0).abs() < 1e-9);
        assert!((l.frame.x - (1920.0 - l.frame.width) / 2.0).abs() < 1e-9);

        p.frame.aspect = "9:16".into();
        let l = Layout::new(&p);
        assert_eq!((l.width, l.height), (1080, 1920));
        assert!((l.frame.width - (1080.0 - 2.0 * pad)).abs() < 1e-9);
        assert!((l.frame.y + l.frame.height / 2.0 - 960.0).abs() < 1e-9);

        p.frame.aspect = "source".into();
        p.crop = Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        };
        assert_eq!(Layout::new(&p).width, 1080);
    }
}
