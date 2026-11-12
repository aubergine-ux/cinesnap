//! CPU frame compositor used for export.
//!
//! Drawing rules (kept identical to the canvas preview in `src/render/`):
//! background → frame shadow → recording (rounded) → annotations → ripples →
//! cursor → webcam → keystrokes → captions. All sizes scale with `Layout::s`
//! (1.0 at 1080p).

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rayon::prelude::*;
use tiny_skia::{
    BlendMode, Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint,
    Path as SkPath, PathBuilder, Pixmap, PixmapMut, Point, Shader, SpreadMode, Stroke, Transform,
};

use crate::layout::{view_rect, Layout};
use crate::motion::KeyMark;
use crate::project::{
    parse_color, Annotation, Background, CaptionStyle, Card, ClipBody, KeystrokeStyle, Project,
    Rect, Shape, Theme, TransitionKind,
};
use crate::text::{self, TextStyle, CENTER_BASELINE, FIRST_BASELINE, LINE_HEIGHT};
use crate::transcript::CaptionLine;

/// Arrow cursor outline in unit coordinates (height = 1, tip at the origin).
pub const ARROW: [(f32, f32); 7] = [
    (0.0, 0.0),
    (0.0, 0.80),
    (0.19, 0.63),
    (0.33, 0.95),
    (0.45, 0.90),
    (0.31, 0.59),
    (0.57, 0.59),
];

pub const RIPPLE_MS: f64 = 500.0;
/// Fade at the start and end of a keystroke label.
pub const KEY_FADE_IN_MS: f64 = 90.0;
pub const KEY_FADE_OUT_MS: f64 = 260.0;
/// How long a card's text takes to appear.
pub const CARD_INTRO_MS: f64 = 500.0;

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

#[derive(Clone, Copy)]
pub struct WebcamFrame<'a> {
    /// Square RGBA frame of side `Compositor::webcam_side()`.
    pub pixels: &'a [u8],
    /// Centre as fractions of the output.
    pub x: f64,
    pub y: f64,
    /// Side as a fraction of the shorter output side.
    pub size: f64,
    pub alpha: f64,
}

pub struct FrameInput<'a> {
    /// Index of the clip being drawn.
    pub clip: usize,
    /// Timeline time (decides which annotations show).
    pub t_ms: f64,
    /// Time since the start of the clip (animates cards).
    pub local_ms: f64,
    /// Output time (karaoke captions).
    pub out_ms: f64,
    /// The cropped recording, RGBA, at the clip's decode size. Unused for
    /// cards.
    pub src: &'a [u8],
    /// Camera: centre in full-source pixels and scale.
    pub cam: (f64, f64, f64),
    pub cursor: Option<CursorState>,
    pub ripples: &'a [RippleState],
    pub webcam: Option<WebcamFrame<'a>>,
    /// Shortcut to show, with its opacity.
    pub key: Option<(&'a str, f64)>,
    pub caption: Option<&'a CaptionLine>,
}

impl<'a> FrameInput<'a> {
    /// A frame of clip 0 with nothing on top of the recording.
    pub fn plain(src: &'a [u8], cam: (f64, f64, f64)) -> Self {
        Self {
            clip: 0,
            t_ms: 0.0,
            local_ms: 0.0,
            out_ms: 0.0,
            src,
            cam,
            cursor: None,
            ripples: &[],
            webcam: None,
            key: None,
            caption: None,
        }
    }
}

/// The shortcut on screen at an output time, with its opacity.
pub fn key_at(keys: &[KeyMark], hold_ms: f64, out_ms: f64) -> Option<(&str, f64)> {
    let n = keys.partition_point(|k| k.out_ms <= out_ms);
    let key = keys.get(n.checked_sub(1)?)?;
    let age = out_ms - key.out_ms;
    if age >= hold_ms {
        return None;
    }
    let alpha = (age / KEY_FADE_IN_MS)
        .min((hold_ms - age) / KEY_FADE_OUT_MS)
        .clamp(0.0, 1.0);
    // A shortcut that replaces another one shows immediately.
    let replaced = n >= 2 && key.out_ms - keys[n - 2].out_ms < hold_ms;
    Some((
        &key.combo,
        if replaced && age < KEY_FADE_IN_MS {
            1.0
        } else {
            alpha
        },
    ))
}

/// The caption line on screen at an output time.
pub fn caption_at(lines: &[CaptionLine], out_ms: f64) -> Option<&CaptionLine> {
    let n = lines.partition_point(|l| l.start_ms <= out_ms);
    lines.get(n.checked_sub(1)?).filter(|l| out_ms < l.end_ms)
}

struct RecStage {
    layout: Layout,
    crop: Rect,
    src_w: usize,
    src_h: usize,
    /// Background with the frame shadow baked in, RGBA.
    base: Arc<Vec<u8>>,
    /// Coverage of the rounded frame, one byte per output pixel.
    frame_mask: Arc<Mask>,
}

struct CardStage {
    base: Arc<Vec<u8>>,
    card: Card,
}

enum Stage {
    Recording(RecStage),
    Card(CardStage),
}

pub struct Compositor {
    width: u32,
    height: u32,
    /// Size unit: 1.0 at 1080p.
    s: f64,
    stages: Vec<Stage>,
    cursor_size: f64,
    ripple_color: [u8; 4],
    draw_ripples: bool,
    annotations: Vec<Annotation>,
    theme: Theme,
    keystrokes: KeystrokeStyle,
    captions: CaptionStyle,
    webcam_side: Option<usize>,
    webcam_shape: String,
}

pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<SkPath> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    if r < 0.01 {
        return Some(PathBuilder::from_rect(tiny_skia::Rect::from_xywh(
            x, y, w, h,
        )?));
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
    p.set_color(Color::from_rgba8(
        c[0],
        c[1],
        c[2],
        (c[3] as f64 * alpha.clamp(0.0, 1.0)).round() as u8,
    ));
    p.anti_alias = true;
    p
}

/// Three box blurs approximate a gaussian of the given sigma.
fn blur_alpha(buf: &mut [u8], w: usize, h: usize, sigma: f64) {
    let r = ((sigma * 3.0f64.sqrt() / 1.5).round() as usize).max(1);
    let mut tmp = vec![0u8; buf.len()];
    let pass =
        |src: &[u8], dst: &mut [u8], len: usize, lines: usize, stride: usize, step: usize| {
            let win = (2 * r + 1) as u32;
            for l in 0..lines {
                let at = |i: isize| {
                    src[l * stride + (i.clamp(0, len as isize - 1) as usize) * step] as u32
                };
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
        return Some(
            img.resize_to_fill(w, h, image::imageops::FilterType::Triangle)
                .to_rgba8()
                .into_raw(),
        );
    }
    // Blur at reduced resolution: far cheaper, visually identical.
    let f = (blur_sigma / 2.5).clamp(1.0, 16.0);
    let (sw, sh) = (
        ((w as f64 / f) as u32).max(2),
        ((h as f64 / f) as u32).max(2),
    );
    let small = img.resize_to_fill(sw, sh, image::imageops::FilterType::Triangle);
    let blurred = small.blur((blur_sigma / f) as f32);
    Some(
        blurred
            .resize_exact(w, h, image::imageops::FilterType::CatmullRom)
            .to_rgba8()
            .into_raw(),
    )
}

fn draw_background(bg: &Background, w: u32, h: u32, s: f64, assets: &Path) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("non-zero output size");
    let full = tiny_skia::Rect::from_xywh(0.0, 0.0, w as f32, h as f32).unwrap();
    let mut paint = Paint::default();
    match bg {
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
            paint.shader =
                LinearGradient::new(p0, p1, stops, SpreadMode::Pad, Transform::identity())
                    .unwrap_or(Shader::SolidColor(color(parse_color(from))));
        }
        Background::Image { path } | Background::Wallpaper { path, .. } => {
            let sigma = match bg {
                Background::Wallpaper { blur, .. } => blur * s,
                _ => 0.0,
            };
            let file = if Path::new(path).is_absolute() {
                path.into()
            } else {
                assets.join(path)
            };
            match load_cover(&file, w, h, sigma) {
                Some(mut px) => {
                    px.as_chunks_mut::<4>()
                        .0
                        .iter_mut()
                        .for_each(|p| p[3] = 255);
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

fn even(v: f64) -> usize {
    (((v / 2.0).round() as usize) * 2).max(2)
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Coverage (0..1) of a rounded rectangle or ellipse at a pixel centre.
#[allow(clippy::too_many_arguments)]
fn shape_coverage(
    px: f64,
    py: f64,
    cx: f64,
    cy: f64,
    hw: f64,
    hh: f64,
    r: f64,
    ellipse: bool,
) -> f64 {
    let (dx, dy) = ((px - cx).abs(), (py - cy).abs());
    let d = if ellipse {
        let k = ((dx / hw.max(0.5)).powi(2) + (dy / hh.max(0.5)).powi(2)).sqrt();
        (k - 1.0) * hw.min(hh)
    } else {
        let r = r.min(hw).min(hh);
        let (qx, qy) = (dx - (hw - r), dy - (hh - r));
        (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r
    };
    (0.5 - d).clamp(0.0, 1.0)
}

impl Compositor {
    /// `assets` is the directory that relative background image paths
    /// resolve against.
    pub fn new(project: &Project, assets: &Path) -> Self {
        let first = Layout::new(project);
        let (width, height, s) = (first.width, first.height, first.s);
        let (w, h) = (width as usize, height as usize);

        let mut backgrounds: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
        let mut background = |bg: &Background| {
            backgrounds
                .entry(serde_json::to_string(bg).unwrap_or_default())
                .or_insert_with(|| Arc::new(draw_background(bg, width, height, s, assets).take()))
                .clone()
        };
        let mut framed: HashMap<String, (Arc<Vec<u8>>, Arc<Mask>)> = HashMap::new();

        let spans = project.spans();
        let mut stages = Vec::with_capacity(project.clips.len());
        for (clip, span) in project.clips.iter().zip(&spans) {
            match &clip.body {
                ClipBody::Card(card) => stages.push(Stage::Card(CardStage {
                    base: background(
                        card.background
                            .as_ref()
                            .unwrap_or(&project.frame.background),
                    ),
                    card: card.clone(),
                })),
                ClipBody::Recording(r) => {
                    let layout = Layout::for_crop(project, r.crop);
                    let f = layout.frame;
                    let key = format!("{:?}", (f.x, f.y, f.width, f.height, layout.radius));
                    let (base, frame_mask) = framed
                        .entry(key)
                        .or_insert_with(|| {
                            let mut base = (*background(&project.frame.background)).clone();
                            // Drop shadow under the frame.
                            if project.frame.shadow > 0.0 {
                                let mut m = Mask::new(width, height).unwrap();
                                let off = 16.0 * s;
                                if let Some(path) = rounded_rect(
                                    f.x as f32,
                                    (f.y + off) as f32,
                                    f.width as f32,
                                    f.height as f32,
                                    layout.radius as f32,
                                ) {
                                    m.fill_path(
                                        &path,
                                        FillRule::Winding,
                                        true,
                                        Transform::identity(),
                                    );
                                }
                                let mut a = m.data().to_vec();
                                blur_alpha(&mut a, w, h, 20.0 * s);
                                let strength = project.frame.shadow * 0.6;
                                for (px, &cov) in base.as_chunks_mut::<4>().0.iter_mut().zip(&a) {
                                    let k = 1.0 - cov as f64 / 255.0 * strength;
                                    for c in &mut px[..3] {
                                        *c = (*c as f64 * k) as u8;
                                    }
                                }
                            }
                            let mut mask = Mask::new(width, height).unwrap();
                            if let Some(path) = rounded_rect(
                                f.x as f32,
                                f.y as f32,
                                f.width as f32,
                                f.height as f32,
                                layout.radius as f32,
                            ) {
                                mask.fill_path(
                                    &path,
                                    FillRule::Winding,
                                    true,
                                    Transform::identity(),
                                );
                            }
                            (Arc::new(base), Arc::new(mask))
                        })
                        .clone();
                    // Decode no more pixels than the deepest zoom can show.
                    let max_scale = project
                        .zooms
                        .iter()
                        .filter(|z| z.start_ms >= span.start_ms - 0.5 && z.start_ms < span.end_ms())
                        .map(|z| z.scale)
                        .fold(1.0, f64::max);
                    let src_w = even((f.width * max_scale).min(r.crop.width));
                    let src_h = even(src_w as f64 * r.crop.height / r.crop.width);
                    stages.push(Stage::Recording(RecStage {
                        layout,
                        crop: r.crop,
                        src_w,
                        src_h,
                        base,
                        frame_mask,
                    }));
                }
            }
        }

        let short = width.min(height) as f64;
        let has_webcam = project
            .clips
            .iter()
            .filter_map(|c| c.recording())
            .any(|r| r.source.webcam.is_some());
        let webcam_side = (has_webcam && project.webcam.show).then(|| {
            let largest = project
                .webcam_keys
                .iter()
                .map(|k| k.size)
                .fold(project.webcam.size, f64::max);
            ((largest * short).round() as usize).clamp(2, short as usize)
        });

        Self {
            width,
            height,
            s,
            stages,
            cursor_size: project.cursor.size,
            ripple_color: parse_color(&project.cursor.ripple_color),
            draw_ripples: project.cursor.ripples,
            annotations: project.annotations.clone(),
            theme: project.theme.clone(),
            keystrokes: project.keystrokes.clone(),
            captions: project.captions.clone(),
            webcam_side,
            webcam_shape: project.webcam.shape.clone(),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Layout of a recording clip.
    pub fn layout(&self, clip: usize) -> Option<&Layout> {
        match self.stages.get(clip)? {
            Stage::Recording(r) => Some(&r.layout),
            Stage::Card(_) => None,
        }
    }

    /// Size of the cropped frames to pass in for a recording clip.
    pub fn decode_size(&self, clip: usize) -> Option<(usize, usize)> {
        match self.stages.get(clip)? {
            Stage::Recording(r) => Some((r.src_w, r.src_h)),
            Stage::Card(_) => None,
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.width as usize * self.height as usize * 4
    }

    /// Side of the square webcam frames to supply, if the overlay is enabled.
    pub fn webcam_side(&self) -> Option<usize> {
        self.webcam_side
    }

    /// Map a point in full-source pixels of a clip to output pixels.
    pub fn map(&self, clip: usize, cam: (f64, f64, f64), x: f64, y: f64) -> (f64, f64) {
        match &self.stages[clip] {
            Stage::Recording(r) => map_point(r, cam, x, y),
            Stage::Card(_) => (0.0, 0.0),
        }
    }

    pub fn render(&self, out: &mut [u8], input: &FrameInput) {
        assert_eq!(out.len(), self.frame_bytes());
        match &self.stages[input.clip] {
            Stage::Card(c) => {
                out.copy_from_slice(&c.base);
                self.draw_card(out, &c.card, input.local_ms);
            }
            Stage::Recording(st) => {
                assert_eq!(input.src.len(), st.src_w * st.src_h * 4);
                out.copy_from_slice(&st.base);
                self.draw_video(st, out, input.src, input.cam);
                for a in &self.annotations {
                    let alpha = a.alpha(input.t_ms);
                    if alpha > 0.0 {
                        self.draw_annotation(st, out, a, alpha, input.cam);
                    }
                }
                self.draw_pointer(st, out, input);
            }
        }
        if let Some(cam) = &input.webcam {
            self.draw_webcam(out, cam);
        }
        if let Some((combo, alpha)) = input.key.filter(|_| self.keystrokes.show) {
            self.draw_keys(out, combo, alpha);
        }
        if let Some(line) = input.caption.filter(|_| self.captions.show) {
            self.draw_caption(out, line, input.out_ms);
        }
    }

    fn pixmap<'a>(&self, out: &'a mut [u8]) -> PixmapMut<'a> {
        PixmapMut::from_bytes(out, self.width, self.height).unwrap()
    }

    fn fill_rounded(
        &self,
        out: &mut [u8],
        b: (f64, f64, f64, f64),
        r: f64,
        c: [u8; 4],
        alpha: f64,
    ) {
        if let Some(path) = rounded_rect(b.0 as f32, b.1 as f32, b.2 as f32, b.3 as f32, r as f32) {
            self.pixmap(out).fill_path(
                &path,
                &solid(c, alpha),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        out: &mut [u8],
        mask: Option<&[u8]>,
        style: TextStyle,
        s: &str,
        x: f64,
        baseline: f64,
        c: [u8; 4],
        alpha: f64,
    ) {
        text::draw(
            out,
            self.width as usize,
            self.height as usize,
            mask,
            style,
            s,
            x as f32,
            baseline as f32,
            c,
            alpha,
        );
    }

    fn draw_pointer(&self, st: &RecStage, out: &mut [u8], input: &FrameInput) {
        let s = self.s * input.cam.2.sqrt();
        let mut pm = PixmapMut::from_bytes(out, self.width, self.height).unwrap();
        let clip = Some(&*st.frame_mask);

        if self.draw_ripples {
            for r in input.ripples {
                let p = r.age_ms / RIPPLE_MS;
                if !(0.0..1.0).contains(&p) {
                    continue;
                }
                let (x, y) = map_point(st, input.cam, r.x, r.y);
                let radius = (8.0 + 36.0 * (1.0 - (1.0 - p).powi(3))) * s;
                if let Some(path) = PathBuilder::from_circle(x as f32, y as f32, radius as f32) {
                    let stroke = Stroke {
                        width: (3.0 * self.s) as f32,
                        ..Default::default()
                    };
                    let paint = solid(self.ripple_color, 0.55 * (1.0 - p));
                    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), clip);
                }
            }
        }

        if let Some(c) = input.cursor.filter(|c| c.alpha > 0.004) {
            let (x, y) = map_point(st, input.cam, c.x, c.y);
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
                pm.fill_path(
                    &path,
                    &solid([0, 0, 0, 255], 0.28 * c.alpha),
                    FillRule::Winding,
                    shadow,
                    clip,
                );
                pm.fill_path(
                    &path,
                    &solid([0, 0, 0, 255], c.alpha),
                    FillRule::Winding,
                    Transform::identity(),
                    clip,
                );
                let stroke = Stroke {
                    width: 0.07 * h,
                    line_join: LineJoin::Round,
                    ..Default::default()
                };
                pm.stroke_path(
                    &path,
                    &solid([255, 255, 255, 255], c.alpha),
                    &stroke,
                    Transform::identity(),
                    clip,
                );
            }
        }
    }

    fn draw_annotation(
        &self,
        st: &RecStage,
        out: &mut [u8],
        a: &Annotation,
        alpha: f64,
        cam: (f64, f64, f64),
    ) {
        let unit = self.s * cam.2.sqrt();
        let v = view_rect(st.crop, cam.0, cam.1, cam.2);
        // Output pixels per source pixel.
        let k = st.layout.frame.width / v.width;
        let clip = Some(&*st.frame_mask);
        let rect = |x: f64, y: f64, w: f64, h: f64| {
            let (ox, oy) = map_point(st, cam, x, y);
            (ox, oy, w * k, h * k)
        };
        match &a.shape {
            Shape::Blur {
                x,
                y,
                width,
                height,
                strength,
            } => {
                self.blur_region(st, out, rect(*x, *y, *width, *height), strength * k);
            }
            Shape::Spotlight {
                x,
                y,
                width,
                height,
                dim,
                ellipse,
            } => {
                let (rx, ry, rw, rh) = rect(*x, *y, *width, *height);
                let (cx, cy, hw, hh) = (rx + rw / 2.0, ry + rh / 2.0, rw / 2.0, rh / 2.0);
                let f = st.layout.frame;
                let w = self.width as usize;
                let x0 = f.x.floor().max(0.0) as usize;
                let x1 = ((f.x + f.width).ceil() as usize).min(w);
                let y0 = f.y.floor().max(0.0) as usize;
                let y1 = ((f.y + f.height).ceil() as usize).min(self.height as usize);
                let mask = st.frame_mask.data();
                let radius = 10.0 * unit;
                let strength = (dim * alpha).clamp(0.0, 1.0);
                out.par_chunks_exact_mut(w * 4)
                    .enumerate()
                    .skip(y0)
                    .take(y1.saturating_sub(y0))
                    .for_each(|(py, line)| {
                        for px in x0..x1 {
                            let inside = shape_coverage(
                                px as f64 + 0.5,
                                py as f64 + 0.5,
                                cx,
                                cy,
                                hw,
                                hh,
                                radius,
                                *ellipse,
                            );
                            let d = strength * (1.0 - inside) * mask[py * w + px] as f64 / 255.0;
                            if d > 0.0 {
                                let keep = ((1.0 - d) * 256.0) as u32;
                                for c in &mut line[px * 4..px * 4 + 3] {
                                    *c = ((*c as u32 * keep) >> 8) as u8;
                                }
                            }
                        }
                    });
            }
            Shape::Highlight {
                x,
                y,
                width,
                height,
                color: c,
                opacity,
            } => {
                let (rx, ry, rw, rh) = rect(*x, *y, *width, *height);
                if let Some(path) = rounded_rect(
                    rx as f32,
                    ry as f32,
                    rw as f32,
                    rh as f32,
                    (6.0 * unit) as f32,
                ) {
                    let mut paint = solid(parse_color(c), opacity * alpha);
                    paint.blend_mode = BlendMode::Multiply;
                    self.pixmap(out).fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        clip,
                    );
                }
            }
            Shape::Box {
                x,
                y,
                width,
                height,
                color: c,
                stroke,
                radius,
            } => {
                let (rx, ry, rw, rh) = rect(*x, *y, *width, *height);
                if let Some(path) = rounded_rect(
                    rx as f32,
                    ry as f32,
                    rw as f32,
                    rh as f32,
                    (radius * k) as f32,
                ) {
                    let stroke = Stroke {
                        width: (stroke * unit) as f32,
                        line_join: LineJoin::Round,
                        ..Default::default()
                    };
                    self.pixmap(out).stroke_path(
                        &path,
                        &solid(parse_color(c), alpha),
                        &stroke,
                        Transform::identity(),
                        clip,
                    );
                }
            }
            Shape::Arrow {
                x1,
                y1,
                x2,
                y2,
                color: c,
                width,
            } => {
                let (ax, ay) = map_point(st, cam, *x1, *y1);
                let (bx, by) = map_point(st, cam, *x2, *y2);
                let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
                if len < 1.0 {
                    return;
                }
                let w = width * unit;
                let (dx, dy) = ((bx - ax) / len, (by - ay) / len);
                let head = (4.5 * w).min(0.6 * len);
                let (hx, hy) = (bx - dx * head, by - dy * head);
                let paint = solid(parse_color(c), alpha);
                let mut pm = self.pixmap(out);
                let mut pb = PathBuilder::new();
                pb.move_to(ax as f32, ay as f32);
                pb.line_to((bx - dx * head * 0.8) as f32, (by - dy * head * 0.8) as f32);
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: w as f32,
                        line_cap: LineCap::Round,
                        ..Default::default()
                    };
                    pm.stroke_path(&path, &paint, &stroke, Transform::identity(), clip);
                }
                let half = 0.55 * head;
                let mut pb = PathBuilder::new();
                pb.move_to(bx as f32, by as f32);
                pb.line_to((hx - dy * half) as f32, (hy + dx * half) as f32);
                pb.line_to((hx + dy * half) as f32, (hy - dx * half) as f32);
                pb.close();
                if let Some(path) = pb.finish() {
                    pm.fill_path(
                        &path,
                        &paint,
                        FillRule::Winding,
                        Transform::identity(),
                        clip,
                    );
                }
            }
            Shape::Text {
                x,
                y,
                text,
                size,
                color: c,
                background,
                font,
                bold,
            } => {
                let px = size * unit;
                let style = TextStyle {
                    family: font.as_deref().unwrap_or(&self.theme.font),
                    bold: *bold,
                    px: px as f32,
                };
                let lines: Vec<&str> = text.split('\n').collect();
                let (ox, oy) = map_point(st, cam, *x, *y);
                let (pad_x, pad_y) = if background.is_some() {
                    (0.45 * px, 0.3 * px)
                } else {
                    (0.0, 0.0)
                };
                if let Some(bg) = background {
                    let widest = lines
                        .iter()
                        .map(|l| text::measure(style, l) as f64)
                        .fold(0.0, f64::max);
                    let bw = widest + 2.0 * pad_x;
                    let bh = lines.len() as f64 * LINE_HEIGHT as f64 * px + 2.0 * pad_y;
                    if let Some(path) = rounded_rect(
                        ox as f32,
                        oy as f32,
                        bw as f32,
                        bh as f32,
                        (0.3 * px) as f32,
                    ) {
                        self.pixmap(out).fill_path(
                            &path,
                            &solid(parse_color(bg), alpha),
                            FillRule::Winding,
                            Transform::identity(),
                            clip,
                        );
                    }
                }
                for (i, line) in lines.iter().enumerate() {
                    let baseline =
                        oy + pad_y + (FIRST_BASELINE as f64 + i as f64 * LINE_HEIGHT as f64) * px;
                    self.text(
                        out,
                        Some(st.frame_mask.data()),
                        style,
                        line,
                        ox + pad_x,
                        baseline,
                        parse_color(c),
                        alpha,
                    );
                }
            }
        }
    }

    /// Blur an output-pixel rectangle of the recording so it cannot be read.
    fn blur_region(
        &self,
        st: &RecStage,
        out: &mut [u8],
        (rx, ry, rw, rh): (f64, f64, f64, f64),
        sigma: f64,
    ) {
        let f = st.layout.frame;
        let (w, h) = (self.width as i64, self.height as i64);
        let bound =
            |v: f64, lo: f64, hi: f64, max: i64| (v.clamp(lo, hi).round() as i64).clamp(0, max);
        let x0 = bound(rx, f.x, f.x + f.width, w);
        let x1 = bound(rx + rw, f.x, f.x + f.width, w);
        let y0 = bound(ry, f.y, f.y + f.height, h);
        let y1 = bound(ry + rh, f.y, f.y + f.height, h);
        if x1 - x0 < 2 || y1 - y0 < 2 {
            return;
        }
        let sigma = sigma.max(1.5);
        // Blur a slightly larger area so the edges are as soft as the middle.
        let m = (2.0 * sigma).ceil() as i64;
        let ex0 = bound((x0 - m) as f64, f.x, f.x + f.width, w);
        let ex1 = bound((x1 + m) as f64, f.x, f.x + f.width, w);
        let ey0 = bound((y0 - m) as f64, f.y, f.y + f.height, h);
        let ey1 = bound((y1 + m) as f64, f.y, f.y + f.height, h);
        let (ew, eh) = ((ex1 - ex0) as usize, (ey1 - ey0) as usize);
        let w = w as usize;
        let mut planes: [Vec<u8>; 3] = std::array::from_fn(|_| vec![0u8; ew * eh]);
        for y in 0..eh {
            for x in 0..ew {
                let o = ((ey0 as usize + y) * w + ex0 as usize + x) * 4;
                for (ch, plane) in planes.iter_mut().enumerate() {
                    plane[y * ew + x] = out[o + ch];
                }
            }
        }
        planes
            .par_iter_mut()
            .for_each(|p| blur_alpha(p, ew, eh, sigma));
        let mask = st.frame_mask.data();
        for y in y0 as usize..y1 as usize {
            for x in x0 as usize..x1 as usize {
                let i = y * w + x;
                let (bx, by) = (x - ex0 as usize, y - ey0 as usize);
                let px = [
                    planes[0][by * ew + bx],
                    planes[1][by * ew + bx],
                    planes[2][by * ew + bx],
                    255,
                ];
                blend(&mut out[i * 4..i * 4 + 4], &px, mask[i] as u32);
            }
        }
    }

    fn draw_webcam(&self, out: &mut [u8], cam: &WebcamFrame) {
        let Some(src_side) = self.webcam_side else {
            return;
        };
        if cam.alpha <= 0.004 || cam.pixels.len() != src_side * src_side * 4 {
            return;
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let short = w.min(h) as f64;
        // The bubble shrinks a little as it fades.
        let side = cam.size * short * (0.85 + 0.15 * cam.alpha);
        let (cx, cy) = (cam.x * w as f64, cam.y * h as f64);
        let half = side / 2.0;
        let radius = match self.webcam_shape.as_str() {
            "circle" => half,
            "square" => 0.04 * side,
            _ => 0.18 * side,
        };
        let x0 = (cx - half).floor().max(0.0) as usize;
        let x1 = ((cx + half).ceil().max(0.0) as usize).min(w);
        let y0 = (cy - half).floor().max(0.0) as usize;
        let y1 = ((cy + half).ceil().max(0.0) as usize).min(h);
        let k = src_side as f64 / side;
        let max = (src_side - 1) as f64;
        for y in y0..y1 {
            let sy = (((y as f64 + 0.5) - (cy - half)) * k - 0.5).clamp(0.0, max);
            let (r0, wy) = (sy.floor() as usize, sy.fract());
            let r1 = (r0 + 1).min(src_side - 1);
            for x in x0..x1 {
                let cov = shape_coverage(
                    x as f64 + 0.5,
                    y as f64 + 0.5,
                    cx,
                    cy,
                    half,
                    half,
                    radius,
                    false,
                ) * cam.alpha;
                if cov <= 0.0 {
                    continue;
                }
                let sx = (((x as f64 + 0.5) - (cx - half)) * k - 0.5).clamp(0.0, max);
                let (c0, wx) = (sx.floor() as usize, sx.fract());
                let c1 = (c0 + 1).min(src_side - 1);
                let p =
                    |r: usize, c: usize, ch: usize| cam.pixels[(r * src_side + c) * 4 + ch] as f64;
                let mut px = [0u8; 4];
                for (ch, v) in px.iter_mut().take(3).enumerate() {
                    let top = p(r0, c0, ch) * (1.0 - wx) + p(r0, c1, ch) * wx;
                    let bot = p(r1, c0, ch) * (1.0 - wx) + p(r1, c1, ch) * wx;
                    *v = (top * (1.0 - wy) + bot * wy).round() as u8;
                }
                blend(
                    &mut out[(y * w + x) * 4..(y * w + x) * 4 + 4],
                    &px,
                    (cov * 255.0).round() as u32,
                );
            }
        }
    }

    /// The pressed shortcut as a row of keycaps.
    fn draw_keys(&self, out: &mut [u8], combo: &str, alpha: f64) {
        let keys: Vec<&str> = combo.split('+').filter(|k| !k.is_empty()).collect();
        if keys.is_empty() || alpha <= 0.0 {
            return;
        }
        let (w, h) = (self.width as f64, self.height as f64);
        let cap = 54.0 * self.s * self.keystrokes.size;
        let style = TextStyle {
            family: &self.theme.font,
            bold: true,
            px: (0.46 * cap) as f32,
        };
        let gap = 0.16 * cap;
        let pad = 0.2 * cap;
        let widths: Vec<f64> = keys
            .iter()
            .map(|k| (text::measure(style, k) as f64 + 0.7 * cap).max(cap))
            .collect();
        let total = widths.iter().sum::<f64>() + gap * (keys.len() - 1) as f64 + 2.0 * pad;
        let box_h = cap + 2.0 * pad;
        let margin = 0.045 * h.min(w);
        let pos = self.keystrokes.position.as_str();
        let x = match pos {
            "bottom-left" => margin,
            "bottom-right" => w - margin - total,
            _ => (w - total) / 2.0,
        };
        let y = if pos == "top" {
            margin
        } else {
            h - margin - box_h
        };
        self.fill_rounded(
            out,
            (x, y, total, box_h),
            0.34 * cap,
            parse_color(&self.theme.surface),
            alpha,
        );
        let mut kx = x + pad;
        for (key, kw) in keys.iter().zip(&widths) {
            self.fill_rounded(
                out,
                (kx, y + pad, *kw, cap),
                0.22 * cap,
                [255, 255, 255, 36],
                alpha,
            );
            let tw = text::measure(style, key) as f64;
            let baseline = y + pad + cap / 2.0 + CENTER_BASELINE as f64 * style.px as f64;
            self.text(
                out,
                None,
                style,
                key,
                kx + (kw - tw) / 2.0,
                baseline,
                parse_color(&self.theme.text),
                alpha,
            );
            kx += kw + gap;
        }
    }

    fn draw_caption(&self, out: &mut [u8], line: &CaptionLine, out_ms: f64) {
        let c = &self.captions;
        let (w, h) = (self.width as f64, self.height as f64);
        let words: Vec<String> = line
            .words
            .iter()
            .map(|x| {
                if c.uppercase {
                    x.text.to_uppercase()
                } else {
                    x.text.clone()
                }
            })
            .collect();
        let family = c.font.as_deref().unwrap_or(&self.theme.font);
        let mut px = 44.0 * self.s * c.size;
        let style = |px: f64| TextStyle {
            family,
            bold: true,
            px: px as f32,
        };
        let width = |px: f64| {
            let space = text::measure(style(px), " ") as f64;
            words
                .iter()
                .map(|x| text::measure(style(px), x) as f64)
                .sum::<f64>()
                + space * words.len().saturating_sub(1) as f64
        };
        // Shrink lines that would not fit.
        let limit = 0.9 * w;
        let natural = width(px);
        if natural > limit {
            px *= limit / natural;
        }
        let st = style(px);
        let space = text::measure(st, " ") as f64;
        let total = width(px);
        let (pad_x, pad_y) = (0.5 * px, 0.28 * px);
        let cy = c.y * h;
        let x0 = (w - total) / 2.0;
        if let Some(bg) = &c.background {
            let bh = LINE_HEIGHT as f64 * px + 2.0 * pad_y;
            self.fill_rounded(
                out,
                (x0 - pad_x, cy - bh / 2.0, total + 2.0 * pad_x, bh),
                0.3 * px,
                parse_color(bg),
                1.0,
            );
        }
        let baseline = cy + CENTER_BASELINE as f64 * px;
        // The word being spoken: the last one that has started.
        let current = line.words.iter().rposition(|x| x.start_ms <= out_ms);
        let mut x = x0;
        for (i, (word, timing)) in words.iter().zip(&line.words).enumerate() {
            let (color, alpha) = match c.style.as_str() {
                "karaoke" if current == Some(i) => (&c.highlight, 1.0),
                "pop" => (
                    &c.color,
                    ((out_ms - timing.start_ms) / 90.0).clamp(0.0, 1.0),
                ),
                _ => (&c.color, 1.0),
            };
            self.text(out, None, st, word, x, baseline, parse_color(color), alpha);
            x += text::measure(st, word) as f64 + space;
        }
    }

    fn draw_card(&self, out: &mut [u8], card: &Card, local_ms: f64) {
        let (w, h) = (self.width as f64, self.height as f64);
        let family = card.font.as_deref().unwrap_or(&self.theme.font);
        let left = card.align == "left";
        let limit = if left { 0.8 * w } else { 0.84 * w };
        let fit = |lines: &[&str], px: f64, bold: bool| {
            let widest = lines
                .iter()
                .map(|l| {
                    text::measure(
                        TextStyle {
                            family,
                            bold,
                            px: px as f32,
                        },
                        l,
                    ) as f64
                })
                .fold(0.0, f64::max);
            if widest > limit {
                px * limit / widest
            } else {
                px
            }
        };
        let titles: Vec<&str> = card.title.split('\n').filter(|l| !l.is_empty()).collect();
        let subs: Vec<&str> = card
            .subtitle
            .split('\n')
            .filter(|l| !l.is_empty())
            .collect();
        let tpx = fit(&titles, card.title_size * self.s, true);
        let spx = fit(&subs, 0.42 * card.title_size * self.s, false);
        let lh = LINE_HEIGHT as f64;
        let gap = if titles.is_empty() || subs.is_empty() {
            0.0
        } else {
            0.35 * tpx
        };
        let block = titles.len() as f64 * lh * tpx + gap + subs.len() as f64 * lh * spx;
        let mut y = (h - block) / 2.0;
        let mut draw =
            |lines: &[&str], px: f64, bold: bool, color: &str, delay: f64, y: &mut f64| {
                let p = smoothstep((local_ms - delay) / CARD_INTRO_MS);
                let style = TextStyle {
                    family,
                    bold,
                    px: px as f32,
                };
                for line in lines {
                    let tw = text::measure(style, line) as f64;
                    let x = if left { 0.1 * w } else { (w - tw) / 2.0 };
                    let baseline = *y + FIRST_BASELINE as f64 * px + (1.0 - p) * 24.0 * self.s;
                    self.text(out, None, style, line, x, baseline, parse_color(color), p);
                    *y += lh * px;
                }
            };
        draw(&titles, tpx, true, &card.title_color, 0.0, &mut y);
        y += gap;
        draw(&subs, spx, false, &card.subtitle_color, 150.0, &mut y);
    }

    /// Resample the visible part of the recording into the frame rect.
    fn draw_video(&self, st: &RecStage, out: &mut [u8], src: &[u8], cam: (f64, f64, f64)) {
        let f = st.layout.frame;
        let (w, h) = (self.width as usize, self.height as usize);
        let x0 = (f.x.floor().max(0.0)) as usize;
        let y0 = (f.y.floor().max(0.0)) as usize;
        let x1 = ((f.x + f.width).ceil() as usize).min(w);
        let y1 = ((f.y + f.height).ceil() as usize).min(h);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let v = view_rect(st.crop, cam.0, cam.1, cam.2);
        // Decoded-source pixels per output pixel.
        let kx = st.src_w as f64 / st.crop.width;
        let ky = st.src_h as f64 / st.crop.height;
        let (step_x, step_y) = (v.width * kx / f.width, v.height * ky / f.height);
        // When shrinking noticeably, take 2×2 bilinear taps per pixel so thin
        // lines and text do not shimmer.
        let taps: usize = if step_x.max(step_y) > 1.4 { 2 } else { 1 };

        // Per-axis lookup: (index0, index1, weight 0..256) for each tap.
        let table =
            |o0: usize, o1: usize, origin: f64, view0: f64, step: f64, k: f64, limit: usize| {
                let mut t = Vec::with_capacity((o1 - o0) * taps);
                for o in o0..o1 {
                    for tap in 0..taps {
                        let sub = (tap as f64 + 0.5) / taps as f64;
                        let pos = (view0 * k + (o as f64 + sub - origin) * step - 0.5)
                            .clamp(0.0, (limit - 1) as f64);
                        let i = pos.floor() as usize;
                        t.push((i, (i + 1).min(limit - 1), ((pos - i as f64) * 256.0) as u32));
                    }
                }
                t
            };
        let cols = table(x0, x1, f.x, v.x - st.crop.x, step_x, kx, st.src_w);
        let rows = table(y0, y1, f.y, v.y - st.crop.y, step_y, ky, st.src_h);
        let mask = st.frame_mask.data();
        let sw = st.src_w;
        let shift = if taps == 2 { 18 } else { 16 };

        out.par_chunks_exact_mut(w * 4)
            .enumerate()
            .skip(y0)
            .take(y1 - y0)
            .for_each(|(y, line)| {
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
                    let px = [
                        (acc[0] >> shift) as u8,
                        (acc[1] >> shift) as u8,
                        (acc[2] >> shift) as u8,
                        255,
                    ];
                    blend(&mut line[x * 4..x * 4 + 4], &px, m);
                }
            });
    }
}

fn map_point(st: &RecStage, cam: (f64, f64, f64), x: f64, y: f64) -> (f64, f64) {
    let v = view_rect(st.crop, cam.0, cam.1, cam.2);
    let f = st.layout.frame;
    (
        f.x + (x - v.x) / v.width * f.width,
        f.y + (y - v.y) / v.height * f.height,
    )
}

/// Blend two finished frames for a transition. `p` runs 0..1 from the
/// outgoing frame `a` to the incoming frame `b`.
pub fn blend_transition(
    kind: TransitionKind,
    p: f64,
    a: &[u8],
    b: &[u8],
    out: &mut [u8],
    width: usize,
) {
    let e = smoothstep(p);
    match kind {
        TransitionKind::Cut => out.copy_from_slice(if p < 0.5 { a } else { b }),
        TransitionKind::Crossfade => {
            let wb = (e * 256.0).round() as u32;
            let wa = 256 - wb;
            out.par_chunks_mut(1 << 16)
                .zip(a.par_chunks(1 << 16).zip(b.par_chunks(1 << 16)))
                .for_each(|(o, (a, b))| {
                    for ((o, a), b) in o.iter_mut().zip(a).zip(b) {
                        *o = ((*a as u32 * wa + *b as u32 * wb) >> 8) as u8;
                    }
                });
        }
        // Push through the outgoing frame while the incoming one grows out
        // of its centre.
        TransitionKind::Zoom => {
            let height = out.len() / (width * 4);
            let (cx, cy) = (width as f64 / 2.0, height as f64 / 2.0);
            let sa = 1.0 + 1.2 * e;
            let sb = 0.4 + 0.6 * e;
            let alpha = (e * 1.6).min(1.0);
            let sample = |buf: &[u8], x: f64, y: f64| -> Option<[f64; 3]> {
                if x < 0.0 || y < 0.0 || x > (width - 1) as f64 || y > (height - 1) as f64 {
                    return None;
                }
                let (x0, y0) = (x.floor() as usize, y.floor() as usize);
                let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
                let (fx, fy) = (x.fract(), y.fract());
                let p = |x: usize, y: usize, ch: usize| buf[(y * width + x) * 4 + ch] as f64;
                Some(std::array::from_fn(|ch| {
                    let top = p(x0, y0, ch) * (1.0 - fx) + p(x1, y0, ch) * fx;
                    let bot = p(x0, y1, ch) * (1.0 - fx) + p(x1, y1, ch) * fx;
                    top * (1.0 - fy) + bot * fy
                }))
            };
            out.par_chunks_exact_mut(width * 4)
                .enumerate()
                .for_each(|(y, line)| {
                    let dy = y as f64 + 0.5 - cy;
                    for x in 0..width {
                        let dx = x as f64 + 0.5 - cx;
                        let mut px =
                            sample(a, cx + dx / sa - 0.5, cy + dy / sa - 0.5).unwrap_or([0.0; 3]);
                        if let Some(pb) = sample(b, cx + dx / sb - 0.5, cy + dy / sb - 0.5) {
                            for ch in 0..3 {
                                px[ch] += (pb[ch] - px[ch]) * alpha;
                            }
                        }
                        let o = &mut line[x * 4..x * 4 + 4];
                        for ch in 0..3 {
                            o[ch] = px[ch].round() as u8;
                        }
                        o[3] = 255;
                    }
                });
        }
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
    use crate::project::{Clip, Transition, WebcamTrack};
    use crate::testutil::project;
    use crate::transcript::CaptionWord;

    fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let o = ((y * w + x) * 4) as usize;
        [buf[o], buf[o + 1], buf[o + 2]]
    }

    /// 200×100 source: left half red, right half blue.
    fn source() -> Vec<u8> {
        let mut v = Vec::new();
        for _y in 0..100 {
            for x in 0..200 {
                v.extend(if x < 100 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                });
            }
        }
        v
    }

    fn base(mutate: impl FnOnce(&mut Project)) -> Project {
        let mut p = project(200, 100, 1000.0);
        p.frame.background = Background::Solid {
            color: "#00ff00".into(),
        };
        p.frame.shadow = 0.0;
        p.frame.aspect = "source".into();
        p.export.preset = "720p".into();
        mutate(&mut p);
        p
    }

    fn setup(mutate: impl FnOnce(&mut Project)) -> (Compositor, Vec<u8>) {
        let c = Compositor::new(&base(mutate), Path::new("."));
        let out = vec![0u8; c.frame_bytes()];
        (c, out)
    }

    const CAM: (f64, f64, f64) = (100.0, 50.0, 1.0);

    fn differing(a: &[u8], b: &[u8]) -> usize {
        a.chunks(4).zip(b.chunks(4)).filter(|(a, b)| a != b).count()
    }

    #[test]
    fn frames_video_on_background_with_rounded_corners() {
        let (c, mut out) = setup(|_| {});
        let src = source();
        c.render(&mut out, &FrameInput::plain(&src, CAM));
        let l = *c.layout(0).unwrap();
        assert_eq!((l.width, l.height), (1440, 720));
        assert_eq!(c.decode_size(0), Some((200, 100)));
        assert_eq!(
            px(&out, l.width, 5, 5),
            [0, 255, 0],
            "padding shows background"
        );
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
        c.render(&mut out, &FrameInput::plain(&src, (150.0, 50.0, 2.0)));
        let l = *c.layout(0).unwrap();
        for x in [
            l.frame.x as u32 + 30,
            720,
            (l.frame.x + l.frame.width) as u32 - 30,
        ] {
            assert_eq!(px(&out, l.width, x, 360), [0, 0, 255], "x = {x}");
        }
        // Mapping: the source centre sits at the left edge of the frame.
        let (mx, _) = c.map(0, (150.0, 50.0, 2.0), 100.0, 50.0);
        assert!((mx - l.frame.x).abs() < 1e-6);
    }

    #[test]
    fn crop_restricts_the_source() {
        let (c, mut out) = setup(|p| {
            p.main_mut().crop = Rect {
                x: 100.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            }
        });
        assert_eq!(c.decode_size(0), Some((100, 100)));
        // Frames handed to the compositor are already cropped.
        let src: Vec<u8> = std::iter::repeat_n([0u8, 0, 255, 255], 100 * 100)
            .flatten()
            .collect();
        c.render(&mut out, &FrameInput::plain(&src, (150.0, 50.0, 1.0)));
        assert_eq!((c.width(), c.height()), (720, 720));
        assert_eq!(px(&out, 720, 360, 360), [0, 0, 255]);
    }

    #[test]
    fn cursor_ripple_and_shadow_are_drawn() {
        let (c, mut out) = setup(|p| p.frame.shadow = 1.0);
        let src = source();
        let l = *c.layout(0).unwrap();
        c.render(&mut out, &FrameInput::plain(&src, CAM));
        let plain = out.clone();
        // Shadow darkens the background just below the frame.
        let below = px(&out, l.width, 720, (l.frame.y + l.frame.height) as u32 + 6);
        assert!(below[1] < 235, "{below:?}");

        let ripples = [RippleState {
            age_ms: 250.0,
            x: 150.0,
            y: 50.0,
        }];
        c.render(
            &mut out,
            &FrameInput {
                cursor: Some(CursorState {
                    x: 50.0,
                    y: 50.0,
                    alpha: 1.0,
                    press: 0.0,
                }),
                ripples: &ripples,
                ..FrameInput::plain(&src, CAM)
            },
        );
        let (cx, cy) = c.map(0, CAM, 50.0, 50.0);
        // Inside the arrow body: black. On red elsewhere.
        assert_eq!(px(&out, l.width, cx as u32 + 4, cy as u32 + 14), [0, 0, 0]);
        assert!(differing(&out, &plain) > 300);
        // The ripple ring lightens blue pixels at its radius.
        let (rx, ry) = c.map(0, CAM, 150.0, 50.0);
        let radius = (8.0 + 36.0 * (1.0 - 0.5f64.powi(3))) * l.s;
        let ring = px(&out, l.width, (rx + radius) as u32, ry as u32);
        assert!(ring[0] > 40, "{ring:?}");
    }

    fn webcam_project(p: &mut Project) {
        p.main_mut().source.webcam = Some(WebcamTrack {
            file: "webcam.mp4".into(),
            offset_ms: 0.0,
            width: 640,
            height: 480,
        });
    }

    #[test]
    fn webcam_bubble_follows_its_state_and_shape() {
        let (c, mut out) = setup(webcam_project);
        let src = source();
        let side = c.webcam_side().unwrap();
        let cam_px: Vec<u8> = std::iter::repeat_n([255u8, 255, 0, 255], side * side)
            .flatten()
            .collect();
        let frame = |x: f64, y: f64, size: f64, alpha: f64| WebcamFrame {
            pixels: &cam_px,
            x,
            y,
            size,
            alpha,
        };
        let yellow = [255, 255, 0];
        c.render(
            &mut out,
            &FrameInput {
                webcam: Some(frame(0.5, 0.5, 0.18, 1.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        let half = (0.18 * 720.0 / 2.0) as u32;
        assert_eq!(px(&out, 1440, 720, 360), yellow);
        assert_eq!(px(&out, 1440, 720 + half - 3, 360), yellow);
        assert_ne!(px(&out, 1440, 720 + half + 3, 360), yellow);
        assert_ne!(
            px(&out, 1440, 720 + half - 3, 360 + half - 3),
            yellow,
            "corner is outside the circle"
        );
        // Moved and resized.
        c.render(
            &mut out,
            &FrameInput {
                webcam: Some(frame(0.2, 0.3, 0.1, 1.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        assert_ne!(px(&out, 1440, 720, 360), yellow);
        assert_eq!(px(&out, 1440, 288, 216), yellow);
        // Hidden: nothing drawn. Partly off screen: no panic.
        let mut plain = out.clone();
        c.render(&mut plain, &FrameInput::plain(&src, CAM));
        c.render(
            &mut out,
            &FrameInput {
                webcam: Some(frame(0.5, 0.5, 0.18, 0.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        assert_eq!(differing(&out, &plain), 0);
        c.render(
            &mut out,
            &FrameInput {
                webcam: Some(frame(1.0, 0.0, 0.18, 1.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        assert_eq!(
            px(&out, 1440, 1439, 0),
            yellow,
            "square corner of the frame"
        );

        let (square, mut out) = setup(|p| {
            webcam_project(p);
            p.webcam.shape = "square".into();
        });
        square.render(
            &mut out,
            &FrameInput {
                webcam: Some(frame(0.5, 0.5, 0.18, 1.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        assert_eq!(px(&out, 1440, 720 + half - 6, 360 + half - 6), yellow);
    }

    #[test]
    fn gradient_runs_in_css_direction() {
        let bg = Background::Gradient {
            from: "#000000".into(),
            to: "#ffffff".into(),
            angle: 90.0,
        };
        let pm = draw_background(&bg, 1440, 720, 1.0, Path::new("."));
        let left = px(pm.data(), 1440, 2, 360)[0];
        let right = px(pm.data(), 1440, 1437, 360)[0];
        assert!(left < 10 && right > 245, "{left} {right}");
    }

    fn annotated(shape: Shape) -> (Compositor, Vec<u8>, Vec<u8>) {
        let (c, mut out) = setup(|p| {
            p.annotations.push(Annotation {
                id: "a".into(),
                start_ms: 0.0,
                end_ms: 1000.0,
                shape,
            })
        });
        let src = source();
        c.render(
            &mut out,
            &FrameInput {
                t_ms: 2000.0,
                ..FrameInput::plain(&src, CAM)
            },
        );
        let plain = out.clone();
        c.render(
            &mut out,
            &FrameInput {
                t_ms: 500.0,
                ..FrameInput::plain(&src, CAM)
            },
        );
        (c, out, plain)
    }

    #[test]
    fn blur_hides_the_edge_between_the_halves() {
        let (c, out, plain) = annotated(Shape::Blur {
            x: 80.0,
            y: 20.0,
            width: 40.0,
            height: 40.0,
            strength: 6.0,
        });
        let (x, y) = c.map(0, CAM, 100.0, 40.0);
        let p = px(&out, 1440, x as u32, y as u32);
        assert!(p[0] > 60 && p[0] < 200 && p[2] > 60 && p[2] < 200, "{p:?}");
        // Outside the region nothing changed.
        let (x, y) = c.map(0, CAM, 100.0, 80.0);
        assert_eq!(px(&out, 1440, x as u32 - 40, y as u32), [255, 0, 0]);
        let (ox, oy) = c.map(0, CAM, 80.0, 20.0);
        for (i, (a, b)) in out.chunks(4).zip(plain.chunks(4)).enumerate() {
            if a != b {
                let (x, y) = ((i % 1440) as f64, (i / 1440) as f64);
                assert!(x >= ox - 1.0 && y >= oy - 1.0, "changed outside at {x},{y}");
            }
        }
    }

    #[test]
    fn spotlight_dims_everything_else() {
        let (c, out, _) = annotated(Shape::Spotlight {
            x: 20.0,
            y: 20.0,
            width: 60.0,
            height: 60.0,
            dim: 0.5,
            ellipse: false,
        });
        let at = |x: f64, y: f64| {
            let (x, y) = c.map(0, CAM, x, y);
            px(&out, 1440, x as u32, y as u32)
        };
        assert_eq!(at(50.0, 50.0), [255, 0, 0]);
        let dimmed = at(150.0, 50.0);
        assert!((120..=135).contains(&dimmed[2]), "{dimmed:?}");
        // The padding around the frame is not part of the recording.
        assert_eq!(px(&out, 1440, 5, 5), [0, 255, 0]);
    }

    #[test]
    fn shapes_and_text_are_drawn_inside_the_frame() {
        let (c, out, plain) = annotated(Shape::Box {
            x: 20.0,
            y: 20.0,
            width: 60.0,
            height: 60.0,
            color: "#ffffff".into(),
            stroke: 6.0,
            radius: 0.0,
        });
        let (x, y) = c.map(0, CAM, 20.0, 50.0);
        assert_eq!(px(&out, 1440, x as u32, y as u32), [255, 255, 255]);
        let (x, y) = c.map(0, CAM, 50.0, 50.0);
        assert_eq!(px(&out, 1440, x as u32, y as u32), [255, 0, 0]);
        assert!(differing(&out, &plain) > 500);

        let (c, out, _) = annotated(Shape::Arrow {
            x1: 20.0,
            y1: 50.0,
            x2: 80.0,
            y2: 50.0,
            color: "#ffffff".into(),
            width: 8.0,
        });
        let (x, y) = c.map(0, CAM, 40.0, 50.0);
        assert_eq!(px(&out, 1440, x as u32, y as u32), [255, 255, 255]);
        // The head is wider than the shaft.
        let (hx, hy) = c.map(0, CAM, 80.0, 50.0);
        assert_eq!(
            px(&out, 1440, hx as u32 - 20, hy as u32 + 8),
            [255, 255, 255]
        );
        assert_eq!(px(&out, 1440, x as u32, y as u32 + 8), [255, 0, 0]);

        let (c, out, _) = annotated(Shape::Highlight {
            x: 120.0,
            y: 20.0,
            width: 60.0,
            height: 60.0,
            color: "#ffff00".into(),
            opacity: 1.0,
        });
        // Multiplying blue by yellow leaves black; red is untouched.
        let (x, y) = c.map(0, CAM, 150.0, 50.0);
        assert_eq!(px(&out, 1440, x as u32, y as u32), [0, 0, 0]);

        let (_, out, plain) = annotated(Shape::Text {
            x: 10.0,
            y: 10.0,
            text: "Look\nhere".into(),
            size: 40.0,
            color: "#ffffff".into(),
            background: Some("#000000".into()),
            font: None,
            bold: true,
        });
        let white = out.chunks(4).filter(|p| p[..3] == [255, 255, 255]).count();
        let black = out.chunks(4).filter(|p| p[..3] == [0, 0, 0]).count();
        assert!(white > 400 && black > 3000, "{white} {black}");
        assert!(differing(&out, &plain) > 3000);
    }

    #[test]
    fn annotations_fade_and_blur_does_not() {
        let mut a = Annotation {
            id: "a".into(),
            start_ms: 1000.0,
            end_ms: 3000.0,
            shape: Shape::Box {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                color: "#fff".into(),
                stroke: 1.0,
                radius: 0.0,
            },
        };
        assert_eq!(a.alpha(999.0), 0.0);
        assert_eq!(a.alpha(1000.0), 0.0);
        assert_eq!(a.alpha(1090.0), 0.5);
        assert_eq!(a.alpha(2000.0), 1.0);
        assert_eq!(a.alpha(3000.0), 0.0);
        a.shape = Shape::Blur {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
            strength: 5.0,
        };
        assert_eq!(a.alpha(1000.0), 1.0);
        assert_eq!(a.alpha(2999.0), 1.0);
    }

    #[test]
    fn keystrokes_and_captions_are_drawn_over_the_output() {
        let (c, mut out) = setup(|_| {});
        let src = source();
        c.render(&mut out, &FrameInput::plain(&src, CAM));
        let plain = out.clone();
        c.render(
            &mut out,
            &FrameInput {
                key: Some(("Ctrl+Shift+P", 1.0)),
                ..FrameInput::plain(&src, CAM)
            },
        );
        let changed = differing(&out, &plain);
        assert!(changed > 4000, "{changed}");
        // Bottom centre, nothing in the top half.
        assert_eq!(
            differing(&out[..1440 * 360 * 4], &plain[..1440 * 360 * 4]),
            0
        );
        assert!(px(&out, 1440, 720, 720 - 50) != px(&plain, 1440, 720, 720 - 50));

        let line = CaptionLine {
            start_ms: 0.0,
            end_ms: 1000.0,
            words: ["hello", "there"]
                .iter()
                .enumerate()
                .map(|(i, w)| CaptionWord {
                    text: w.to_string(),
                    start_ms: i as f64 * 400.0,
                    end_ms: i as f64 * 400.0 + 300.0,
                })
                .collect(),
        };
        let mut first = plain.clone();
        c.render(
            &mut first,
            &FrameInput {
                caption: Some(&line),
                out_ms: 100.0,
                ..FrameInput::plain(&src, CAM)
            },
        );
        let mut second = plain.clone();
        c.render(
            &mut second,
            &FrameInput {
                caption: Some(&line),
                out_ms: 500.0,
                ..FrameInput::plain(&src, CAM)
            },
        );
        assert!(differing(&first, &plain) > 4000);
        // The highlighted word moved from the left half to the right half.
        let highlight = |buf: &[u8], left: bool| {
            buf.chunks(4)
                .enumerate()
                .filter(|(i, p)| (i % 1440 < 720) == left && p[..3] == [0xff, 0xd8, 0x4a])
                .count()
        };
        assert!(highlight(&first, true) > 50 && highlight(&first, false) == 0);
        assert!(highlight(&second, false) > 50 && highlight(&second, true) == 0);
    }

    #[test]
    fn key_labels_hold_fade_and_replace() {
        let keys = vec![
            KeyMark {
                out_ms: 1000.0,
                combo: "Ctrl+K".into(),
            },
            KeyMark {
                out_ms: 1500.0,
                combo: "Enter".into(),
            },
        ];
        assert_eq!(key_at(&keys, 1000.0, 900.0), None);
        assert_eq!(key_at(&keys, 1000.0, 1045.0), Some(("Ctrl+K", 0.5)));
        assert_eq!(key_at(&keys, 1000.0, 1400.0), Some(("Ctrl+K", 1.0)));
        // Replaces the previous one without fading in again.
        assert_eq!(key_at(&keys, 1000.0, 1510.0), Some(("Enter", 1.0)));
        assert_eq!(key_at(&keys, 1000.0, 2370.0), Some(("Enter", 0.5)));
        assert_eq!(key_at(&keys, 1000.0, 2500.0), None);
        assert_eq!(key_at(&[], 1000.0, 0.0), None);
    }

    #[test]
    fn cards_show_their_text_after_the_intro() {
        let (c, mut out) = setup(|p| {
            p.clips.push(Clip {
                id: "card".into(),
                transition: Transition::default(),
                body: ClipBody::Card(Card {
                    title: "Hello".into(),
                    subtitle: "A subtitle".into(),
                    background: Some(Background::Solid {
                        color: "#000000".into(),
                    }),
                    ..Card::default()
                }),
            })
        });
        let card = |local_ms: f64, out: &mut Vec<u8>| {
            c.render(
                out,
                &FrameInput {
                    clip: 1,
                    local_ms,
                    ..FrameInput::plain(&[], CAM)
                },
            )
        };
        card(0.0, &mut out);
        assert!(
            out.chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "text starts invisible"
        );
        card(2000.0, &mut out);
        let white = out.chunks(4).filter(|p| p[..3] == [255, 255, 255]).count();
        assert!(white > 1500, "{white}");
        // Title is centred: about as much ink left of centre as right of it.
        let (mut left, mut right) = (0, 0);
        for (i, p) in out.chunks(4).enumerate() {
            if p[0] > 128 {
                if i % 1440 < 720 {
                    left += 1
                } else {
                    right += 1
                }
            }
        }
        assert!(
            (left as f64 / right as f64 - 1.0).abs() < 0.35,
            "{left} {right}"
        );
    }

    #[test]
    fn transitions_start_at_one_frame_and_end_at_the_other() {
        let (w, h) = (64usize, 32usize);
        let a: Vec<u8> = std::iter::repeat_n([200u8, 0, 0, 255], w * h)
            .flatten()
            .collect();
        let b: Vec<u8> = std::iter::repeat_n([0u8, 0, 200, 255], w * h)
            .flatten()
            .collect();
        let mut out = vec![0u8; a.len()];
        for kind in [TransitionKind::Crossfade, TransitionKind::Zoom] {
            blend_transition(kind, 0.0, &a, &b, &mut out, w);
            assert_eq!(px(&out, w as u32, 32, 16), [200, 0, 0], "{kind:?}");
            blend_transition(kind, 1.0, &a, &b, &mut out, w);
            assert_eq!(px(&out, w as u32, 32, 16), [0, 0, 200], "{kind:?}");
            assert_eq!(px(&out, w as u32, 1, 1), [0, 0, 200], "{kind:?}");
            blend_transition(kind, 0.5, &a, &b, &mut out, w);
            let mid = px(&out, w as u32, 32, 16);
            assert!(mid[0] > 20 && mid[2] > 20, "{kind:?} {mid:?}");
        }
        // Half way through a zoom the incoming frame has not reached the
        // corners yet.
        blend_transition(TransitionKind::Zoom, 0.5, &a, &b, &mut out, w);
        assert_eq!(px(&out, w as u32, 1, 1), [200, 0, 0]);
        blend_transition(TransitionKind::Cut, 0.4, &a, &b, &mut out, w);
        assert_eq!(out, a);
    }
}
