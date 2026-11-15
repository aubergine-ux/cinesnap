//! Output geometry shared by the preview and the exporter.

use crate::presets::SizeRule;
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
    /// Corner radius of the frame in output pixels (the largest corner).
    pub radius: f64,
    /// Corner radii of the frame: top left, top right, bottom right, bottom
    /// left. A window title bar squares off the top.
    pub radii: [f64; 4],
    /// Size of the part of the source shown at zoom 1: the whole crop, or
    /// with reframing a window of the frame's shape inside it.
    pub view_w: f64,
    pub view_h: f64,
    pub device: DeviceKind,
    /// Outline of the device or window around the frame (the frame itself
    /// without one), and its corner radius.
    pub body: Rect,
    pub body_radius: f64,
    pub insets: Insets,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    None,
    Browser,
    MacOs,
    Windows,
    Laptop,
    Phone,
}

impl DeviceKind {
    pub fn parse(s: &str) -> Self {
        match s {
            "browser" => Self::Browser,
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            "laptop" => Self::Laptop,
            "phone" => Self::Phone,
            _ => Self::None,
        }
    }

    /// A window with a title bar along the top.
    pub fn is_window(self) -> bool {
        matches!(self, Self::Browser | Self::MacOs | Self::Windows)
    }
}

/// Room a device takes around the recording, in output pixels.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Insets {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Height of a window title bar (a browser's also holds the address).
pub const TITLE_BAR: f64 = 40.0;
pub const BROWSER_BAR: f64 = 60.0;
/// Laptop and phone proportions, as fractions of the shorter side of the
/// space they are placed in.
pub const LAPTOP_BEZEL: f64 = 0.03;
pub const LAPTOP_CHIN: f64 = 0.045;
pub const LAPTOP_BASE: f64 = 0.04;
pub const LAPTOP_OVERHANG: f64 = 0.07;
pub const PHONE_BEZEL: f64 = 0.022;

/// Insets of a device in a space whose shorter side is `short`, at unit
/// size `s`.
pub fn device_insets(kind: DeviceKind, s: f64, short: f64) -> Insets {
    match kind {
        DeviceKind::None => Insets::default(),
        DeviceKind::Browser => Insets {
            top: BROWSER_BAR * s,
            ..Default::default()
        },
        DeviceKind::MacOs | DeviceKind::Windows => Insets {
            top: TITLE_BAR * s,
            ..Default::default()
        },
        DeviceKind::Laptop => {
            let side = (LAPTOP_BEZEL + LAPTOP_OVERHANG) * short;
            Insets {
                left: side,
                right: side,
                top: LAPTOP_BEZEL * short,
                bottom: (LAPTOP_CHIN + LAPTOP_BASE) * short,
            }
        }
        DeviceKind::Phone => {
            let b = PHONE_BEZEL * short;
            Insets {
                left: b,
                top: b,
                right: b,
                bottom: b,
            }
        }
    }
}

/// The largest window of aspect `ratio` (width / height) inside `crop`.
pub fn fit_inside(crop: Rect, ratio: f64) -> (f64, f64) {
    if crop.width / crop.height > ratio {
        (crop.height * ratio, crop.height)
    } else {
        (crop.width, crop.width / ratio)
    }
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
    let rule = crate::presets::find(preset).map_or(SizeRule::Short(1080), |p| p.size);
    match rule {
        SizeRule::Width(w) => (even(w as f64), even(w as f64 / ratio)),
        SizeRule::Short(p) if ratio >= 1.0 => (even(p as f64 * ratio), even(p as f64)),
        SizeRule::Short(p) => (even(p as f64), even(p as f64 / ratio)),
    }
}

/// Output frame rate: some presets carry their own, the rest use the setting.
pub fn output_fps(project: &Project) -> u32 {
    let preset = crate::presets::resolve(&project.export.preset, &project.export.format);
    preset.fps.unwrap_or(project.export.fps).max(1)
}

/// Crop of the first recording on the timeline: the one that decides the
/// output shape when the aspect is `source`.
pub fn main_crop(project: &Project) -> Rect {
    project
        .clips
        .iter()
        .find_map(|c| c.recording())
        .map_or(project.crop, |r| r.crop)
}

/// Width / height of the output.
pub fn output_ratio(project: &Project) -> f64 {
    let preset = crate::presets::resolve(&project.export.preset, &project.export.format);
    aspect_ratio(
        preset.aspect.unwrap_or(&project.frame.aspect),
        main_crop(project),
    )
}

impl Layout {
    /// Layout for the first recording on the timeline.
    pub fn new(project: &Project) -> Self {
        Self::for_crop(project, main_crop(project))
    }

    /// Layout for a recording with the given crop. The output size is the
    /// same for every clip; only the frame inside it differs.
    pub fn for_crop(project: &Project, crop: Rect) -> Self {
        let (width, height) = output_size(&project.export.preset, output_ratio(project));
        Self::with_size(project, crop, width, height)
    }

    pub fn with_size(project: &Project, crop: Rect, width: u32, height: u32) -> Self {
        let (w, h) = (width as f64, height as f64);
        let short = w.min(h);
        let s = short / 1080.0;
        let pad = project.frame.padding * short;
        let (aw, ah) = ((w - 2.0 * pad).max(2.0), (h - 2.0 * pad).max(2.0));
        let device = DeviceKind::parse(&project.frame.device.kind);
        let ins = device_insets(device, s, aw.min(ah));
        let (iw, ih) = (
            (aw - ins.left - ins.right).max(2.0),
            (ah - ins.top - ins.bottom).max(2.0),
        );
        let crop_ratio = crop.width / crop.height;
        // Reframing fills the space; otherwise the recording keeps its shape.
        let (fw, fh) = if project.reframe.enabled {
            (iw, ih)
        } else if iw / ih > crop_ratio {
            (ih * crop_ratio, ih)
        } else {
            (iw, iw / crop_ratio)
        };
        let (view_w, view_h) = if project.reframe.enabled {
            fit_inside(crop, fw / fh)
        } else {
            (crop.width, crop.height)
        };
        let (tw, th) = (fw + ins.left + ins.right, fh + ins.top + ins.bottom);
        let frame = Rect {
            x: (w - tw) / 2.0 + ins.left,
            y: (h - th) / 2.0 + ins.top,
            width: fw,
            height: fh,
        };
        let r = (project.frame.radius * s).min(fw.min(fh) / 2.0);
        let (radii, body, body_radius) = match device {
            DeviceKind::None => ([r; 4], frame, r),
            d if d.is_window() => {
                let body = Rect {
                    y: frame.y - ins.top,
                    height: fh + ins.top,
                    ..frame
                };
                let r = r.max(8.0 * s).min(fw.min(fh) / 2.0);
                ([0.0, 0.0, r, r], body, r)
            }
            DeviceKind::Laptop => {
                let b = LAPTOP_BEZEL * aw.min(ah);
                let lid = Rect {
                    x: frame.x - b,
                    y: frame.y - b,
                    width: fw + 2.0 * b,
                    height: fh + b + LAPTOP_CHIN * aw.min(ah),
                };
                ([0.4 * b; 4], lid, 0.8 * b)
            }
            _ => {
                let b = PHONE_BEZEL * aw.min(ah);
                let screen = 0.11 * fw.min(fh);
                let body = Rect {
                    x: frame.x - b,
                    y: frame.y - b,
                    width: fw + 2.0 * b,
                    height: fh + 2.0 * b,
                };
                ([screen; 4], body, screen + b)
            }
        };
        Self {
            width,
            height,
            s,
            frame,
            radius: radii.iter().copied().fold(0.0, f64::max),
            radii,
            view_w,
            view_h,
            device,
            body,
            body_radius,
            insets: ins,
        }
    }

    /// The part of the source visible for a camera state.
    pub fn view(&self, cx: f64, cy: f64, scale: f64) -> Rect {
        let scale = scale.max(1.0);
        let (w, h) = (self.view_w / scale, self.view_h / scale);
        Rect {
            x: cx - w / 2.0,
            y: cy - h / 2.0,
            width: w,
            height: h,
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
        p.main_mut().crop = Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        };
        assert_eq!(Layout::new(&p).width, 1080);
    }

    #[test]
    fn without_reframing_the_whole_crop_is_the_view() {
        let p = project(2560, 1440, 1000.0);
        let l = Layout::new(&p);
        assert_eq!((l.view_w, l.view_h), (2560.0, 1440.0));
        assert_eq!(l.view(1280.0, 720.0, 2.0).width, 1280.0);
        assert_eq!(l.body, l.frame);
        assert_eq!(l.radii, [l.radius; 4]);
    }

    #[test]
    fn reframing_fills_a_vertical_output_with_a_window_of_its_shape() {
        let mut p = project(2560, 1440, 1000.0);
        p.frame.aspect = "9:16".into();
        p.reframe.enabled = true;
        let l = Layout::new(&p);
        let pad = 0.06 * 1080.0;
        // The frame fills the padded output…
        assert!((l.frame.width - (1080.0 - 2.0 * pad)).abs() < 1e-9);
        assert!((l.frame.height - (1920.0 - 2.0 * pad)).abs() < 1e-9);
        // …and shows a full-height slice of the recording with its shape.
        assert_eq!(l.view_h, 1440.0);
        assert!((l.view_w / l.view_h - l.frame.width / l.frame.height).abs() < 1e-9);
        // A square output of the same recording.
        p.frame.aspect = "1:1".into();
        let l = Layout::new(&p);
        assert!((l.view_w - 1440.0).abs() < 1e-9 && l.view_h == 1440.0);
        // A tall recording in a wide output keeps its full width.
        let (w, h) = fit_inside(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1080.0,
                height: 1920.0,
            },
            16.0 / 9.0,
        );
        assert!((w - 1080.0).abs() < 1e-9 && (h - 607.5).abs() < 1e-9);
    }

    #[test]
    fn window_chrome_sits_on_top_and_squares_the_frame_corners() {
        let mut p = project(1920, 1080, 1000.0);
        p.frame.device.kind = "macos".into();
        let l = Layout::new(&p);
        assert_eq!(l.insets.top, TITLE_BAR);
        assert_eq!(l.body.y, l.frame.y - TITLE_BAR);
        assert_eq!(l.body.height, l.frame.height + TITLE_BAR);
        assert_eq!((l.radii[0], l.radii[1]), (0.0, 0.0));
        assert!(l.radii[2] > 0.0);
        // Frame plus title bar is centred and fits the padded space.
        let pad = 0.06 * 1080.0;
        assert!(l.body.height <= 1080.0 - 2.0 * pad + 1e-9);
        assert!((l.body.y + l.body.height / 2.0 - 540.0).abs() < 1e-9);
        assert!((l.frame.width / l.frame.height - 16.0 / 9.0).abs() < 1e-9);
    }

    #[test]
    fn laptops_and_phones_fit_with_their_bezels() {
        for kind in ["laptop", "phone", "browser", "windows"] {
            for aspect in ["16:9", "9:16", "1:1"] {
                let mut p = project(2560, 1440, 1000.0);
                p.frame.device.kind = kind.into();
                p.frame.aspect = aspect.into();
                let l = Layout::new(&p);
                let pad = p.frame.padding * l.width.min(l.height) as f64;
                let total_w = l.frame.width + l.insets.left + l.insets.right;
                let total_h = l.frame.height + l.insets.top + l.insets.bottom;
                assert!(
                    total_w <= l.width as f64 - 2.0 * pad + 1e-6,
                    "{kind} {aspect}"
                );
                assert!(
                    total_h <= l.height as f64 - 2.0 * pad + 1e-6,
                    "{kind} {aspect}"
                );
                assert!(
                    l.body.x <= l.frame.x && l.body.y <= l.frame.y,
                    "{kind} {aspect}"
                );
                assert!(l.frame.x + l.frame.width <= l.body.x + l.body.width + 1e-9);
            }
        }
    }
}
