//! Device frames around the recording: browser and desktop windows, a
//! laptop, a phone. Drawn from `Layout`, the same way as in
//! `src/render/device.ts`.

use tiny_skia::{FillRule, LineCap, Paint, PathBuilder, PixmapMut, Stroke, Transform};

use crate::layout::{DeviceKind, Layout, LAPTOP_BEZEL};
use crate::project::{parse_color, Device};
use crate::text::{self, TextStyle};

pub struct Palette {
    pub body: &'static str,
    pub bar: &'static str,
    pub field: &'static str,
    pub text: &'static str,
    pub base: &'static str,
    pub base_top: &'static str,
}

pub fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            body: "#1c1d23",
            bar: "#26272f",
            field: "#33353f",
            text: "#d4d6de",
            base: "#2a2b31",
            base_top: "#3d3e45",
        }
    } else {
        Palette {
            body: "#f2f2f5",
            bar: "#e4e5ea",
            field: "#ffffff",
            text: "#3b3d45",
            base: "#c6c7cd",
            base_top: "#dcdde2",
        }
    }
}

pub const TRAFFIC: [&str; 3] = ["#ff5f57", "#febc2e", "#28c840"];
pub const TITLE_PX: f64 = 14.0;

fn paint(hex: &str, alpha: f64) -> Paint<'static> {
    let c = parse_color(hex);
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], (c[3] as f64 * alpha) as u8);
    p.anti_alias = true;
    p
}

/// A rectangle with its own radius per corner: top left, top right, bottom
/// right, bottom left.
pub fn rounded_corners(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Option<tiny_skia::Path> {
    let lim = (w / 2.0).min(h / 2.0);
    let [tl, tr, br, bl] = r.map(|v| v.clamp(0.0, lim));
    const K: f32 = 0.447_715_25; // 1 - kappa
    let mut p = PathBuilder::new();
    p.move_to(x + tl, y);
    p.line_to(x + w - tr, y);
    p.cubic_to(x + w - tr * K, y, x + w, y + tr * K, x + w, y + tr);
    p.line_to(x + w, y + h - br);
    p.cubic_to(
        x + w,
        y + h - br * K,
        x + w - br * K,
        y + h,
        x + w - br,
        y + h,
    );
    p.line_to(x + bl, y + h);
    p.cubic_to(x + bl * K, y + h, x, y + h - bl * K, x, y + h - bl);
    p.line_to(x, y + tl);
    p.cubic_to(x, y + tl * K, x + tl * K, y, x + tl, y);
    p.close();
    p.finish()
}

fn fill(pm: &mut PixmapMut, path: Option<tiny_skia::Path>, hex: &str, alpha: f64) {
    if let Some(path) = path {
        pm.fill_path(
            &path,
            &paint(hex, alpha),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn circle(pm: &mut PixmapMut, x: f64, y: f64, r: f64, hex: &str) {
    fill(
        pm,
        PathBuilder::from_circle(x as f32, y as f32, r as f32),
        hex,
        1.0,
    );
}

/// Everything of the device that does not change from frame to frame:
/// the body, title bar, controls and title. Drawn under the recording.
pub fn draw_body(out: &mut [u8], width: u32, height: u32, l: &Layout, d: &Device) {
    if l.device == DeviceKind::None {
        return;
    }
    let pal = palette(d.dark);
    let s = l.s;
    let (f, b) = (l.frame, l.body);
    let r = l.body_radius as f32;
    {
        let mut pm = PixmapMut::from_bytes(out, width, height).unwrap();
        match l.device {
            DeviceKind::Laptop => {
                fill(
                    &mut pm,
                    rounded_corners(
                        b.x as f32,
                        b.y as f32,
                        b.width as f32,
                        b.height as f32,
                        [r; 4],
                    ),
                    pal.body,
                    1.0,
                );
                let short = (f.width + l.insets.left + l.insets.right)
                    .min(f.height + l.insets.top + l.insets.bottom);
                let bezel = f.x - b.x;
                circle(
                    &mut pm,
                    b.x + b.width / 2.0,
                    b.y + bezel / 2.0,
                    0.18 * bezel.min(LAPTOP_BEZEL * short),
                    "#0a0a0c",
                );
                let (bx0, bx1) = (f.x - l.insets.left, f.x + f.width + l.insets.right);
                let (by0, by1) = (b.y + b.height, f.y + f.height + l.insets.bottom);
                let bh = (by1 - by0) as f32;
                fill(
                    &mut pm,
                    rounded_corners(
                        bx0 as f32,
                        by0 as f32,
                        (bx1 - bx0) as f32,
                        bh,
                        [0.0, 0.0, 0.6 * bh, 0.6 * bh],
                    ),
                    pal.base,
                    1.0,
                );
                fill(
                    &mut pm,
                    rounded_corners(
                        bx0 as f32,
                        by0 as f32,
                        (bx1 - bx0) as f32,
                        0.28 * bh,
                        [0.0; 4],
                    ),
                    pal.base_top,
                    1.0,
                );
                let nw = 0.14 * b.width;
                fill(
                    &mut pm,
                    rounded_corners(
                        (b.x + (b.width - nw) / 2.0) as f32,
                        by0 as f32,
                        nw as f32,
                        0.32 * bh,
                        [0.0, 0.0, 0.3 * bh, 0.3 * bh],
                    ),
                    pal.body,
                    0.6,
                );
            }
            DeviceKind::Phone => {
                fill(
                    &mut pm,
                    rounded_corners(
                        b.x as f32,
                        b.y as f32,
                        b.width as f32,
                        b.height as f32,
                        [r; 4],
                    ),
                    pal.body,
                    1.0,
                );
            }
            _ => {
                fill(
                    &mut pm,
                    rounded_corners(
                        b.x as f32,
                        b.y as f32,
                        b.width as f32,
                        b.height as f32,
                        [r, r, r, r],
                    ),
                    pal.bar,
                    1.0,
                );
                let bar_h = f.y - b.y;
                let cy = b.y + bar_h / 2.0;
                match l.device {
                    DeviceKind::Windows => {
                        let stroke = Stroke {
                            width: (1.2 * s) as f32,
                            line_cap: LineCap::Square,
                            ..Default::default()
                        };
                        let g = 5.0 * s;
                        for i in 0..3 {
                            let x = b.x + b.width - (46.0 * (i as f64 + 0.5)) * s;
                            let mut p = PathBuilder::new();
                            match i {
                                // close, maximise, minimise from the right
                                0 => {
                                    p.move_to((x - g) as f32, (cy - g) as f32);
                                    p.line_to((x + g) as f32, (cy + g) as f32);
                                    p.move_to((x + g) as f32, (cy - g) as f32);
                                    p.line_to((x - g) as f32, (cy + g) as f32);
                                }
                                1 => {
                                    if let Some(rect) = tiny_skia::Rect::from_xywh(
                                        (x - g) as f32,
                                        (cy - g) as f32,
                                        (2.0 * g) as f32,
                                        (2.0 * g) as f32,
                                    ) {
                                        p.push_rect(rect);
                                    }
                                }
                                _ => {
                                    p.move_to((x - g) as f32, cy as f32);
                                    p.line_to((x + g) as f32, cy as f32);
                                }
                            }
                            if let Some(path) = p.finish() {
                                pm.stroke_path(
                                    &path,
                                    &paint(pal.text, 1.0),
                                    &stroke,
                                    Transform::identity(),
                                    None,
                                );
                            }
                        }
                    }
                    _ => {
                        for (i, c) in TRAFFIC.iter().enumerate() {
                            circle(&mut pm, b.x + (20.0 + 20.0 * i as f64) * s, cy, 6.0 * s, c);
                        }
                    }
                }
                if l.device == DeviceKind::Browser {
                    let (fx, fw, fh) = (b.x + 90.0 * s, b.width - 180.0 * s, 32.0 * s);
                    if fw > 40.0 * s {
                        fill(
                            &mut pm,
                            rounded_corners(
                                fx as f32,
                                (cy - fh / 2.0) as f32,
                                fw as f32,
                                fh as f32,
                                [(fh / 2.0) as f32; 4],
                            ),
                            pal.field,
                            1.0,
                        );
                    }
                }
            }
        }
    }
    if !l.device.is_window() || d.title.trim().is_empty() {
        return;
    }
    let px = TITLE_PX * s;
    let style = TextStyle {
        family: "Inter",
        bold: false,
        px: px as f32,
    };
    let cy = b.y + (f.y - b.y) / 2.0;
    let baseline = cy + text::CENTER_BASELINE as f64 * px;
    let x = match l.device {
        DeviceKind::Browser => b.x + 106.0 * s,
        DeviceKind::Windows => b.x + 16.0 * s,
        _ => b.x + (b.width - text::measure(style, &d.title) as f64) / 2.0,
    };
    text::draw(
        out,
        width as usize,
        height as usize,
        None,
        style,
        &d.title,
        x as f32,
        baseline as f32,
        parse_color(pal.text),
        1.0,
    );
}

/// Parts drawn over the recording: a phone's camera cut-out.
pub fn draw_over(out: &mut [u8], width: u32, height: u32, l: &Layout) {
    if l.device != DeviceKind::Phone {
        return;
    }
    let f = l.frame;
    let m = f.width.min(f.height);
    let (w, h) = (0.3 * m, 0.085 * m);
    let mut pm = PixmapMut::from_bytes(out, width, height).unwrap();
    fill(
        &mut pm,
        rounded_corners(
            (f.x + (f.width - w) / 2.0) as f32,
            (f.y + 0.035 * m) as f32,
            w as f32,
            h as f32,
            [(h / 2.0) as f32; 4],
        ),
        "#000000",
        1.0,
    );
}
