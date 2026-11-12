//! CPU frame compositor used for export.
//!
//! Drawing rules (kept identical to the canvas preview in `src/render/`):
//! background → frame shadow → recording (rounded) → ripples → cursor →
//! webcam. All sizes scale with `Layout::s` (1.0 at 1080p).

use std::path::Path;

use rayon::prelude::*;
use tiny_skia::{
    Color, FillRule, GradientStop, LineJoin, LinearGradient, Mask, Paint, Path as SkPath, PathBuilder,
    Pixmap, PixmapMut, Point, Shader, SpreadMode, Stroke, Transform,
};

use crate::layout::{view_rect, Layout};
use crate::project::{parse_color, Background, Project, Rect};

/// Arrow cursor outline in unit coordinates (height = 1, tip at the origin).
pub const ARROW: [(f32, f32); 7] =
    [(0.0, 0.0), (0.0, 0.80), (0.19, 0.63), (0.33, 0.95), (0.45, 0.90), (0.31, 0.59), (0.57, 0.59)];

pub const RIPPLE_MS: f64 = 500.0;

#[derive(Clone, Copy, Debug)]
pub struct CursorState {
    pub x: f64,
    pub y: f64,
    pub alpha: f64,
    pub press: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct RippleState {
    pub age_ms: f64,
    pub x: f64,
    pub y: f64,
}

pub struct FrameInput<'a> {
    /// The cropped recording, RGBA, `src_w × src_h`.
    pub src: &'a [u8],
    /// Camera: centre in full-source pixels and scale.
    pub cam: (f64, f64, f64),
    pub cursor: Option<CursorState>,
    pub ripples: &'a [RippleState],
    /// Square RGBA webcam frame of side `Compositor::webcam_side()`.
    pub webcam: Option<&'a [u8]>,
}

struct WebcamSlot {
    x: usize,
    y: usize,
    side: usize,
    mask: Vec<u8>,
}

pub struct Compositor {
    layout: Layout,
    crop: Rect,
    src_w: usize,
    src_h: usize,
    /// Background with the frame shadow baked in, RGBA.
    base: Vec<u8>,
    /// Coverage of the rounded frame, one byte per output pixel.
    frame_mask: Mask,
    cursor_size: f64,
    ripple_color: [u8; 4],
    draw_ripples: bool,
    webcam: Option<WebcamSlot>,
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<SkPath> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r < 0.01 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(x, y, w, h)?));
    }
    let k = r * 0.447_715_25; // r * (1 - kappa)
    let mut p = PathBuilder::new();
    p.move_to(x + r, y);
    p.line_to(x + w - r, y);
    p.cubic_to(x + w - k, y, x + w, y + k, x + w, y + r);
    p.line_to(x + w, y + h - r);
    p.cubic_to(x + w, y + h - k, x + w - k, y + h, x + w - r, y + h);
    p.line_to(x + r, y + h);
    p.cubic_to(x + k, y + h, x, y + h - k, x, y + h - r);
    p.line_to(x, y + r);
    p.cubic_to(x, y + k, x + k, y, x + r, y);
    p.close();
    p.finish()
}

fn color(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

fn solid(c: [u8; 4], alpha: f64) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(c[0], c[1], c[2], (c[3] as f64 * alpha.clamp(0.0, 1.0)).round() as u8));
    p.anti_alias = true;
    p
}

/// Three box blurs approximate a gaussian of the given sigma.
fn blur_alpha(buf: &mut [u8], w: usize, h: usize, sigma: f64) {
    let r = ((sigma * 3.0f64.sqrt() / 1.5).round() as usize).max(1);
    let mut tmp = vec![0u8; buf.len()];
    let pass = |src: &[u8], dst: &mut [u8], len: usize, lines: usize, stride: usize, step: usize| {
        let win = (2 * r + 1) as u32;
        for l in 0..lines {
            let at = |i: isize| src[l * stride + (i.clamp(0, len as isize - 1) as usize) * step] as u32;
            let mut sum: u32 = (-(r as isize)..=r as isize).map(at).sum();
            for i in 0..len {
                dst[l * stride + i * step] = (sum / win) as u8;
                sum += at(i as isize + r as isize + 1);
                sum -= at(i as isize - r as isize);
            }
        }
    };
    for _ in 0..3 {
        pass(buf, &mut tmp, w, h, w, 1); // horizontal
        pass(&tmp, buf, h, w, 1, w); // vertical
    }
}

fn load_cover(path: &Path, w: u32, h: u32, blur_sigma: f64) -> Option<Vec<u8>> {
    let img = image::open(path).ok()?;
    if blur_sigma < 0.5 {
        return Some(img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8().into_raw());
    }
    // Blur at reduced resolution: far cheaper, visually identical.
    let f = (blur_sigma / 2.5).clamp(1.0, 16.0);
    let (sw, sh) = (((w as f64 / f) as u32).max(2), ((h as f64 / f) as u32).max(2));
    let small = img.resize_to_fill(sw, sh, image::imageops::FilterType::Triangle);
    let blurred = small.blur((blur_sigma / f) as f32);
    Some(blurred.resize_exact(w, h, image::imageops::FilterType::CatmullRom).to_rgba8().into_raw())
}

fn draw_background(project: &Project, layout: &Layout, assets: &Path) -> Pixmap {
    let (w, h) = (layout.width, layout.height);
    let mut pm = Pixmap::new(w, h).expect("non-zero output size");
    let full = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
    let mut paint = Paint::default();
    match &project.frame.background {
        Background::Solid { color: c } => paint.set_color(color(parse_color(c))),
        Background::Gradient { from, to, angle } => {
            // CSS semantics: 0deg points up, 90deg points right.
            let a = angle.to_radians();
            let (dx, dy) = (a.sin(), -a.cos());
            let len = (w as f64 * dx).abs() + (h as f64 * dy).abs();
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            let p0 = Point::from_xy((cx - dx * len / 2.0) as f32, (cy - dy * len / 2.0) as f32);
            let p1 = Point::from_xy((cx + dx * len / 2.0) as f32, (cy + dy * len / 2.0) as f32);
            let stops = vec![
                GradientStop::new(0.0, color(parse_color(from))),
                GradientStop::new(1.0, color(parse_color(to))),
            ];
            paint.shader = LinearGradient::new(p0, p1, stops, SpreadMode::Pad, Transform::identity())
                .unwrap_or(Shader::SolidColor(color(parse_color(from))));
        }
        Background::Image { path } | Background::Wallpaper { path, .. } => {
            let sigma = match &project.frame.background {
                Background::Wallpaper { blur, .. } => blur * layout.s,
                _ => 0.0,
            };
            let file = if Path::new(path).is_absolute() { path.into() } else { assets.join(path) };
            match load_cover(&file, w, h, sigma) {
                Some(mut px) => {
                    px.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p[3] = 255);
                    pm.data_mut().copy_from_slice(&px);
                    return pm;
                }
                None => paint.set_color(Color::from_rgba8(17, 17, 20, 255)),
            }
        }
    }
    pm.fill_rect(full, &paint, Transform::identity(), None);
    pm
}

impl Compositor {
    /// `src_w × src_h` is the size of the (already cropped) frames that will
    /// be passed to [`render`](Self::render). `assets` is the directory that
    /// relative background image paths resolve against.
    pub fn new(project: &Project, layout: Layout, src_w: usize, src_h: usize, assets: &Path) -> Self {
        let (w, h) = (layout.width as usize, layout.height as usize);
        let f = layout.frame;
        let mut base = draw_background(project, &layout, assets);

        // Drop shadow under the frame.
        if project.frame.shadow > 0.0 {
            let mut m = Mask::new(layout.width, layout.height).unwrap();
            let off = 16.0 * layout.s;
            if let Some(path) =
                rounded_rect(f.x as f32, (f.y + off) as f32, f.width as f32, f.height as f32, layout.radius as f32)
            {
                m.fill_path(&path, FillRule::Winding, true, Transform::identity());
            }
            let mut a = m.data().to_vec();
            blur_alpha(&mut a, w, h, 20.0 * layout.s);
            let strength = project.frame.shadow * 0.6;
            for (px, &cov) in base.data_mut().as_chunks_mut::<4>().0.iter_mut().zip(&a) {
                let k = 1.0 - cov as f64 / 255.0 * strength;
                for c in &mut px[..3] {
                    *c = (*c as f64 * k) as u8;
                }
            }
        }

        let mut frame_mask = Mask::new(layout.width, layout.height).unwrap();
        if let Some(path) =
            rounded_rect(f.x as f32, f.y as f32, f.width as f32, f.height as f32, layout.radius as f32)
        {
            frame_mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
        }

        let short = layout.width.min(layout.height) as f64;
        let webcam = (project.source.webcam.is_some() && project.webcam.show).then(|| {
            let side = ((project.webcam.size * short).round() as usize).clamp(2, short as usize);
            let margin = (0.03 * short).round() as usize;
            let x = if project.webcam.corner.ends_with("left") { margin } else { w.saturating_sub(side + margin) };
            let y = if project.webcam.corner.starts_with("top") { margin } else { h.saturating_sub(side + margin) };
            let mut m = Mask::new(side as u32, side as u32).unwrap();
            let r = if project.webcam.shape == "circle" { side as f32 / 2.0 } else { side as f32 * 0.18 };
            if let Some(path) = rounded_rect(0.0, 0.0, side as f32, side as f32, r) {
                m.fill_path(&path, FillRule::Winding, true, Transform::identity());
            }
            WebcamSlot { x, y, side, mask: m.data().to_vec() }
        });

        Self {
            layout,
            crop: project.crop,
            src_w,
            src_h,
            base: base.take(),
            frame_mask,
            cursor_size: project.cursor.size,
            ripple_color: parse_color(&project.cursor.ripple_color),
            draw_ripples: project.cursor.ripples,
            webcam,
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn frame_bytes(&self) -> usize {
        self.layout.width as usize * self.layout.height as usize * 4
    }

    /// Side of the square webcam frames to supply, if the overlay is enabled.
    pub fn webcam_side(&self) -> Option<usize> {
        self.webcam.as_ref().map(|w| w.side)
    }

    /// Map a point in full-source pixels to output pixels for a camera state.
    pub fn map(&self, cam: (f64, f64, f64), x: f64, y: f64) -> (f64, f64) {
        let v = view_rect(self.crop, cam.0, cam.1, cam.2);
        let f = self.layout.frame;
        (f.x + (x - v.x) / v.width * f.width, f.y + (y - v.y) / v.height * f.height)
    }

    pub fn render(&self, out: &mut [u8], input: &FrameInput) {
        assert_eq!(out.len(), self.frame_bytes());
        assert_eq!(input.src.len(), self.src_w * self.src_h * 4);
        out.copy_from_slice(&self.base);
        self.draw_video(out, input.src, input.cam);

        let s = self.layout.s * input.cam.2.sqrt();
        let mut pm = PixmapMut::from_bytes(out, self.layout.width, self.layout.height).unwrap();
        let clip = Some(&self.frame_mask);

        if self.draw_ripples {
            for r in input.ripples {
                let p = r.age_ms / RIPPLE_MS;
                if !(0.0..1.0).contains(&p) {
                    continue;
                }
                let (x, y) = self.map(input.cam, r.x, r.y);
                let radius = (8.0 + 36.0 * (1.0 - (1.0 - p).powi(3))) * s;
                if let Some(path) = PathBuilder::from_circle(x as f32, y as f32, radius as f32) {
                    let stroke = Stroke { width: (3.0 * self.layout.s) as f32, ..Default::default() };
                    let paint = solid(self.ripple_color, 0.55 * (1.0 - p));
                    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), clip);
                }
            }
        }

        if let Some(c) = input.cursor.filter(|c| c.alpha > 0.004) {
            let (x, y) = self.map(input.cam, c.x, c.y);
            let h = (26.0 * self.cursor_size * s * (1.0 - 0.15 * c.press)) as f32;
            let mut pb = PathBuilder::new();
            for (i, (px, py)) in ARROW.iter().enumerate() {
                let (px, py) = (x as f32 + px * h, y as f32 + py * h);
                if i == 0 {
                    pb.move_to(px, py);
                } else {
                    pb.line_to(px, py);
                }
            }
            pb.close();
            if let Some(path) = pb.finish() {
                let shadow = Transform::from_translate(0.0, 0.07 * h);
                pm.fill_path(&path, &solid([0, 0, 0, 255], 0.28 * c.alpha), FillRule::Winding, shadow, clip);
                pm.fill_path(&path, &solid([0, 0, 0, 255], c.alpha), FillRule::Winding, Transform::identity(), clip);
                let stroke = Stroke { width: 0.07 * h, line_join: LineJoin::Round, ..Default::default() };
                pm.stroke_path(&path, &solid([255, 255, 255, 255], c.alpha), &stroke, Transform::identity(), clip);
            }
        }

        if let (Some(slot), Some(cam)) = (&self.webcam, input.webcam) {
            if cam.len() == slot.side * slot.side * 4 {
                let w = self.layout.width as usize;
                for row in 0..slot.side {
                    let o = ((slot.y + row) * w + slot.x) * 4;
                    let dst = &mut out[o..o + slot.side * 4];
                    let src = &cam[row * slot.side * 4..(row + 1) * slot.side * 4];
                    let m = &slot.mask[row * slot.side..(row + 1) * slot.side];
                    for ((d, s), &m) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.chunks_exact(4)).zip(m) {
                        blend(d, s, m as u32);
                    }
                }
            }
        }
    }

    /// Resample the visible part of the recording into the frame rect.
    fn draw_video(&self, out: &mut [u8], src: &[u8], cam: (f64, f64, f64)) {
        let f = self.layout.frame;
        let (w, h) = (self.layout.width as usize, self.layout.height as usize);
        let x0 = (f.x.floor().max(0.0)) as usize;
        let y0 = (f.y.floor().max(0.0)) as usize;
        let x1 = ((f.x + f.width).ceil() as usize).min(w);
        let y1 = ((f.y + f.height).ceil() as usize).min(h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let v = view_rect(self.crop, cam.0, cam.1, cam.2);
        // Decoded-source pixels per output pixel.
        let kx = self.src_w as f64 / self.crop.width;
        let ky = self.src_h as f64 / self.crop.height;
        let (step_x, step_y) = (v.width * kx / f.width, v.height * ky / f.height);
        // When shrinking noticeably, take 2×2 bilinear taps per pixel so thin
        // lines and text do not shimmer.
        let taps: usize = if step_x.max(step_y) > 1.4 { 2 } else { 1 };

        // Per-axis lookup: (index0, index1, weight 0..256) for each tap.
        let table = |o0: usize, o1: usize, origin: f64, view0: f64, step: f64, k: f64, limit: usize| {
            let mut t = Vec::with_capacity((o1 - o0) * taps);
            for o in o0..o1 {
                for tap in 0..taps {
                    let sub = (tap as f64 + 0.5) / taps as f64;
                    let pos = (view0 * k + (o as f64 + sub - origin) * step - 0.5).clamp(0.0, (limit - 1) as f64);
                    let i = pos.floor() as usize;
                    t.push((i, (i + 1).min(limit - 1), ((pos - i as f64) * 256.0) as u32));
                }
            }
            t
        };
        let cols = table(x0, x1, f.x, v.x - self.crop.x, step_x, kx, self.src_w);
        let rows = table(y0, y1, f.y, v.y - self.crop.y, step_y, ky, self.src_h);
        let mask = self.frame_mask.data();
        let sw = self.src_w;
        let shift = if taps == 2 { 18 } else { 16 };

        out.par_chunks_exact_mut(w * 4).enumerate().skip(y0).take(y1 - y0).for_each(|(y, line)| {
            let mrow = &mask[y * w..(y + 1) * w];
            let rtaps = &rows[(y - y0) * taps..(y - y0 + 1) * taps];
            for x in x0..x1 {
                let m = mrow[x] as u32;
                if m == 0 {
                    continue;
                }
                let mut acc = [0u32; 3];
                for &(r0, r1, wy) in rtaps {
                    for &(c0, c1, wx) in &cols[(x - x0) * taps..(x - x0 + 1) * taps] {
                        let p00 = &src[(r0 * sw + c0) * 4..];
                        let p01 = &src[(r0 * sw + c1) * 4..];
                        let p10 = &src[(r1 * sw + c0) * 4..];
                        let p11 = &src[(r1 * sw + c1) * 4..];
                        for ch in 0..3 {
                            let top = p00[ch] as u32 * (256 - wx) + p01[ch] as u32 * wx;
                            let bot = p10[ch] as u32 * (256 - wx) + p11[ch] as u32 * wx;
                            acc[ch] += top * (256 - wy) + bot * wy;
                        }
                    }
                }
                let px = [(acc[0] >> shift) as u8, (acc[1] >> shift) as u8, (acc[2] >> shift) as u8, 255];
                blend(&mut line[x * 4..x * 4 + 4], &px, m);
            }
        });
    }
}

#[inline]
fn blend(dst: &mut [u8], src: &[u8], coverage: u32) {
    if coverage >= 255 {
        dst[..3].copy_from_slice(&src[..3]);
    } else if coverage > 0 {
        for ch in 0..3 {
            let (d, s) = (dst[ch] as u32, src[ch] as u32);
            dst[ch] = ((d * (255 - coverage) + s * coverage + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::project;

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let o = ((y * w + x) * 4) as usize;
        [buf[o], buf[o + 1], buf[o + 2]]
    }

    /// 200×100 source: left half red, right half blue.
    fn source() -> Vec<u8> {
        let mut v = Vec::new();
        for _y in 0..100 {
            for x in 0..200 {
                v.extend(if x < 100 { [255, 0, 0, 255] } else { [0, 0, 255, 255] });
            }
        }
        v
    }

    fn setup(mutate: impl FnOnce(&mut Project)) -> (Compositor, Vec<u8>) {
        let mut p = project(200, 100, 1000.0);
        p.frame.background = Background::Solid { color: "#00ff00".into() };
        p.frame.shadow = 0.0;
        p.frame.aspect = "source".into();
        p.export.preset = "720p".into();
        mutate(&mut p);
        let c = Compositor::new(&p, Layout::new(&p), 200, 100, Path::new("."));
        let out = vec![0u8; c.frame_bytes()];
        (c, out)
    }

    fn input<'a>(src: &'a [u8], cam: (f64, f64, f64)) -> FrameInput<'a> {
        FrameInput { src, cam, cursor: None, ripples: &[], webcam: None }
    }

    #[test]
    fn frames_video_on_background_with_rounded_corners() {
        let (c, mut out) = setup(|_| {});
        let src = source();
        c.render(&mut out, &input(&src, (100.0, 50.0, 1.0)));
        let l = *c.layout();
        assert_eq!((l.width, l.height), (1440, 720));
        assert_eq!(px(&out, l.width, 5, 5), [0, 255, 0], "padding shows background");
        assert_eq!(px(&out, l.width, 400, 360), [255, 0, 0]);
        assert_eq!(px(&out, l.width, 1000, 360), [0, 0, 255]);
        // The very corner of the frame is rounded away.
        let (fx, fy) = (l.frame.x.ceil() as u32, l.frame.y.ceil() as u32);
        assert_eq!(px(&out, l.width, fx, fy), [0, 255, 0]);
        assert_eq!(px(&out, l.width, fx + 20, fy + 20), [255, 0, 0]);
    }

    #[test]
    fn zoom_shows_only_the_focused_part() {
        let (c, mut out) = setup(|_| {});
        let src = source();
        // 2× centred on the blue half: everything in the frame is blue.
        c.render(&mut out, &input(&src, (150.0, 50.0, 2.0)));
        let l = *c.layout();
        for x in [l.frame.x as u32 + 30, 720, (l.frame.x + l.frame.width) as u32 - 30] {
            assert_eq!(px(&out, l.width, x, 360), [0, 0, 255], "x = {x}");
        }
        // Mapping: the source centre sits at the left edge of the frame.
        let (mx, _) = c.map((150.0, 50.0, 2.0), 100.0, 50.0);
        assert!((mx - l.frame.x).abs() < 1e-6);
    }

    #[test]
    fn crop_restricts_the_source() {
        let (c, mut out) = setup(|p| p.crop = Rect { x: 100.0, y: 0.0, width: 100.0, height: 100.0 });
        // Frames handed to the compositor are already cropped.
        let src: Vec<u8> = std::iter::repeat_n([0u8, 0, 255, 255], 100 * 100).flatten().collect();
        let c2 = Compositor { src_w: 100, src_h: 100, ..c };
        out.resize(c2.frame_bytes(), 0);
        c2.render(&mut out, &input(&src, (150.0, 50.0, 1.0)));
        let l = *c2.layout();
        assert_eq!((l.width, l.height), (720, 720));
        assert_eq!(px(&out, l.width, 360, 360), [0, 0, 255]);
    }

    #[test]
    fn cursor_ripple_and_shadow_are_drawn() {
        let (c, mut out) = setup(|p| p.frame.shadow = 1.0);
        let src = source();
        let l = *c.layout();
        let cam = (100.0, 50.0, 1.0);
        c.render(&mut out, &input(&src, cam));
        let plain = out.clone();
        // Shadow darkens the background just below the frame.
        let below = px(&out, l.width, 720, (l.frame.y + l.frame.height) as u32 + 6);
        assert!(below[1] < 235, "{below:?}");

        let cursor = CursorState { x: 50.0, y: 50.0, alpha: 1.0, press: 0.0 };
        let ripples = [RippleState { age_ms: 250.0, x: 150.0, y: 50.0 }];
        c.render(&mut out, &FrameInput { src: &src, cam, cursor: Some(cursor), ripples: &ripples, webcam: None });
        let (cx, cy) = c.map(cam, 50.0, 50.0);
        // Inside the arrow body: black. On red elsewhere.
        assert_eq!(px(&out, l.width, cx as u32 + 4, cy as u32 + 14), [0, 0, 0]);
        let changed = out.chunks(4).zip(plain.chunks(4)).filter(|(a, b)| a != b).count();
        assert!(changed > 300, "cursor and ripple touched {changed} px");
        // The ripple ring lightens blue pixels at its radius.
        let (rx, ry) = c.map(cam, 150.0, 50.0);
        let radius = (8.0 + 36.0 * (1.0 - 0.5f64.powi(3))) * l.s;
        let ring = px(&out, l.width, (rx + radius) as u32, ry as u32);
        assert!(ring[0] > 40, "{ring:?}");
    }

    #[test]
    fn webcam_overlay_is_masked_to_a_circle() {
        let (c, mut out) = setup(|p| {
            p.source.webcam = Some(crate::project::WebcamTrack {
                file: "webcam.mp4".into(), offset_ms: 0.0, width: 640, height: 480,
            });
        });
        let src = source();
        let side = c.webcam_side().unwrap();
        let cam_px: Vec<u8> = std::iter::repeat_n([255u8, 255, 0, 255], side * side).flatten().collect();
        c.render(&mut out, &FrameInput { webcam: Some(&cam_px), ..input(&src, (100.0, 50.0, 1.0)) });
        let l = *c.layout();
        let margin = (0.03 * 720.0f64).round() as u32;
        let (x, y) = (l.width - margin - side as u32, l.height - margin - side as u32);
        assert_eq!(px(&out, l.width, x + side as u32 / 2, y + side as u32 / 2), [255, 255, 0]);
        assert_ne!(px(&out, l.width, x + 1, y + 1), [255, 255, 0], "corner is outside the circle");
    }

    #[test]
    fn gradient_runs_in_css_direction() {
        let (c, _) = setup(|p| {
            p.frame.background = Background::Gradient { from: "#000000".into(), to: "#ffffff".into(), angle: 90.0 };
        });
        let l = *c.layout();
        let left = px(&c.base, l.width, 2, 360)[0];
        let right = px(&c.base, l.width, l.width - 3, 360)[0];
        assert!(left < 10 && right > 245, "{left} {right}");
    }
}
