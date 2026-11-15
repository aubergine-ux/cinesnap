//! What can be recorded on this machine: screens, windows, microphones,
//! webcams, sound outputs and apps playing sound, plus what the session lets
//! us observe about the cursor.

use std::path::Path;
use std::process::Command;

#[cfg(not(target_os = "linux"))]
use cinesnap_core::ffmpeg;
use cinesnap_core::project::Rect;
use serde::Serialize;

pub fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
            || std::env::var_os("WAYLAND_DISPLAY").is_some())
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if is_wayland() {
        "linux-wayland"
    } else {
        "linux-x11"
    }
}

#[derive(Serialize, Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// A monitor. `x`, `y`, `width` and `height` are in video pixels; `origin`
/// and `scale` convert a global pointer position into them.
#[derive(Serialize, Clone, Debug)]
pub struct Screen {
    pub id: String,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// In HDR mode; recorded in 10 bit and converted (Windows).
    pub hdr: bool,
    #[serde(skip)]
    pub origin: (f64, f64),
    #[serde(skip)]
    pub scale: f64,
    /// The grabber's number for it (Desktop Duplication output on Windows).
    #[serde(skip)]
    pub output: usize,
    #[serde(skip)]
    pub sdr_white_nits: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Window {
    pub id: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Sources {
    pub portal: bool,
    pub screens: Vec<Screen>,
    pub windows: Vec<Window>,
    pub mics: Vec<Device>,
    pub webcams: Vec<Device>,
    /// Sound outputs whose playback can be recorded; empty if none can.
    pub outputs: Vec<Device>,
    /// Apps playing sound that can be recorded on their own.
    pub apps: Vec<Device>,
}

pub fn screens() -> Vec<Screen> {
    let Ok(monitors) = xcap::Monitor::all() else {
        return vec![];
    };
    #[cfg(target_os = "windows")]
    let dxgi = crate::dxgi::outputs();
    monitors
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let (x, y, w, h) = (m.x().ok()?, m.y().ok()?, m.width().ok()?, m.height().ok()?);
            // Only macOS reports geometry in points; elsewhere it is pixels.
            let scale = if cfg!(target_os = "macos") {
                m.scale_factor().unwrap_or(1.0) as f64
            } else {
                1.0
            };
            let name = m
                .friendly_name()
                .or_else(|_| m.name())
                .unwrap_or_else(|_| format!("Screen {}", i + 1));
            #[allow(unused_mut)]
            let mut screen = Screen {
                id: i.to_string(),
                name,
                x: (x as f64 * scale).round() as i32,
                y: (y as f64 * scale).round() as i32,
                width: (w as f64 * scale).round() as u32,
                height: (h as f64 * scale).round() as u32,
                hdr: false,
                origin: (x as f64, y as f64),
                scale,
                output: i,
                sdr_white_nits: cinesnap_core::tonemap::REFERENCE_NITS,
            };
            // The monitor list and Desktop Duplication may count screens in
            // a different order; their positions agree.
            #[cfg(target_os = "windows")]
            if let Some(o) = dxgi.iter().find(|o| (o.x, o.y) == (x, y)) {
                screen.output = o.index;
                screen.hdr = o.hdr;
                screen.sdr_white_nits = o.sdr_white_nits;
            }
            Some(screen)
        })
        .collect()
}

pub fn windows() -> Vec<Window> {
    let Ok(all) = xcap::Window::all() else {
        return vec![];
    };
    all.iter()
        .filter_map(|w| {
            let title = w.title().ok()?;
            let usable = !title.trim().is_empty()
                && title != "Cinesnap"
                && !w.is_minimized().unwrap_or(false)
                && w.width().unwrap_or(0) >= 120
                && w.height().unwrap_or(0) >= 80;
            if !usable {
                return None;
            }
            Some(Window {
                id: w.id().ok()?.to_string(),
                title,
            })
        })
        .collect()
}

/// The screen a window is on and the window's rectangle within that screen,
/// in video pixels (clamped to the screen).
pub fn window_area(id: &str, screens: &[Screen]) -> Option<(usize, Rect)> {
    let all = xcap::Window::all().ok()?;
    let w = all
        .iter()
        .find(|w| w.id().is_ok_and(|i| i.to_string() == id))?;
    let (x, y) = (w.x().ok()? as f64, w.y().ok()? as f64);
    let (width, height) = (w.width().ok()? as f64, w.height().ok()? as f64);
    let (cx, cy) = (x + width / 2.0, y + height / 2.0);
    let (index, s) = screens.iter().enumerate().find(|(_, s)| {
        let (sw, sh) = (s.width as f64 / s.scale, s.height as f64 / s.scale);
        cx >= s.origin.0 && cx < s.origin.0 + sw && cy >= s.origin.1 && cy < s.origin.1 + sh
    })?;
    let x0 = ((x - s.origin.0) * s.scale).max(0.0);
    let y0 = ((y - s.origin.1) * s.scale).max(0.0);
    let x1 = ((x + width - s.origin.0) * s.scale).min(s.width as f64);
    let y1 = ((y + height - s.origin.1) * s.scale).min(s.height as f64);
    (x1 - x0 >= 16.0 && y1 - y0 >= 16.0).then_some((
        index,
        Rect {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        },
    ))
}

#[cfg(not(target_os = "linux"))]
fn stderr_of(mut cmd: Command) -> String {
    cmd.output()
        .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
        .unwrap_or_default()
}

/// Parse `ffmpeg -list_devices` output for avfoundation (`[n] Name` under a
/// "video devices" / "audio devices" heading).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn parse_avfoundation(text: &str, audio: bool) -> Vec<Device> {
    let mut in_section = false;
    let mut out = Vec::new();
    for line in text.lines() {
        if line.contains("AVFoundation video devices") {
            in_section = !audio;
        } else if line.contains("AVFoundation audio devices") {
            in_section = audio;
        } else if in_section {
            // "[AVFoundation indev @ 0x...] [0] FaceTime HD Camera"
            let Some(rest) = line.rsplit_once("] [").map(|(_, r)| r) else {
                continue;
            };
            let Some((index, name)) = rest.split_once("] ") else {
                continue;
            };
            if index.parse::<u32>().is_ok() && !name.starts_with("Capture screen") {
                out.push(Device {
                    id: index.into(),
                    name: name.trim().into(),
                });
            }
        }
    }
    out
}

/// Parse `ffmpeg -list_devices true -f dshow` output: `"Name" (video)`.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub fn parse_dshow(text: &str, audio: bool) -> Vec<Device> {
    let kind = if audio { "(audio)" } else { "(video)" };
    text.lines()
        .filter(|l| l.trim_end().ends_with(kind))
        .filter_map(|l| {
            let name = l.split('"').nth(1)?;
            Some(Device {
                id: name.into(),
                name: name.into(),
            })
        })
        .collect()
}

/// Parse `pactl list sources`: `Name:` / `Description:` pairs. Monitors
/// (what an output plays) are the ones `monitors` asks for.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_pactl(text: &str, monitors: bool) -> Vec<Device> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(n) = line.strip_prefix("Name: ") {
            name = Some(n.to_owned());
        } else if let Some(d) = line.strip_prefix("Description: ") {
            if let Some(n) = name.take().filter(|n| n.ends_with(".monitor") == monitors) {
                out.push(Device {
                    id: n,
                    name: d.strip_prefix("Monitor of ").unwrap_or(d).to_owned(),
                });
            }
        }
    }
    out
}

#[cfg(target_os = "linux")]
fn pactl_sources() -> Option<String> {
    let out = Command::new("pactl")
        .args(["list", "sources"])
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Parse `pw-dump`: the audio streams apps are playing, by object serial.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn parse_pw_dump(text: &str) -> Vec<Device> {
    let Ok(serde_json::Value::Array(objects)) = serde_json::from_str(text) else {
        return vec![];
    };
    let mut out = Vec::new();
    for o in &objects {
        let props = &o["info"]["props"];
        if props["media.class"] != "Stream/Output/Audio" {
            continue;
        }
        let serial = match &props["object.serial"] {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            _ => continue,
        };
        let app = props["application.name"]
            .as_str()
            .or(props["node.name"].as_str())
            .unwrap_or("App");
        let name = match props["media.name"].as_str() {
            Some(m) if !m.is_empty() && m != app => format!("{app} — {m}"),
            _ => app.to_owned(),
        };
        out.push(Device { id: serial, name });
    }
    out
}

/// Sound outputs whose playback can be recorded.
pub fn outputs() -> Vec<Device> {
    #[cfg(target_os = "linux")]
    {
        let Some(text) = pactl_sources() else {
            return vec![];
        };
        let mut list = vec![Device {
            id: "@DEFAULT_MONITOR@".into(),
            name: "Default output".into(),
        }];
        list.extend(parse_pactl(&text, true));
        list
    }
    // WASAPI loopback and Core Audio taps record the default output.
    #[cfg(not(target_os = "linux"))]
    {
        vec![Device {
            id: "default".into(),
            name: "Default output".into(),
        }]
    }
}

/// Apps that are playing sound (Linux) or have a window (Windows), whose
/// sound can be recorded on its own.
pub fn apps() -> Vec<Device> {
    #[cfg(target_os = "linux")]
    {
        Command::new("pw-dump")
            .output()
            .map(|o| parse_pw_dump(&String::from_utf8_lossy(&o.stdout)))
            .unwrap_or_default()
    }
    #[cfg(target_os = "windows")]
    {
        let mut seen = std::collections::HashSet::new();
        let mut list = Vec::new();
        for w in xcap::Window::all().unwrap_or_default() {
            let (Ok(pid), Ok(app)) = (w.pid(), w.app_name()) else {
                continue;
            };
            if pid != std::process::id() && !app.is_empty() && seen.insert(pid) {
                list.push(Device {
                    id: pid.to_string(),
                    name: app,
                });
            }
        }
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        list
    }
    #[cfg(target_os = "macos")]
    {
        vec![]
    }
}

#[allow(unused_variables)]
pub fn mics(ffmpeg: &Path) -> Vec<Device> {
    #[cfg(target_os = "linux")]
    {
        let mut list = vec![Device {
            id: "default".into(),
            name: "Default microphone".into(),
        }];
        if let Some(text) = pactl_sources() {
            list.extend(parse_pactl(&text, false));
        }
        list
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-f", "avfoundation", "-list_devices", "true", "-i", ""]);
        parse_avfoundation(&stderr_of(c), true)
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"]);
        parse_dshow(&stderr_of(c), true)
    }
}

#[allow(unused_variables)]
pub fn webcams(ffmpeg: &Path) -> Vec<Device> {
    #[cfg(target_os = "linux")]
    {
        let mut list = Vec::new();
        let Ok(dir) = std::fs::read_dir("/sys/class/video4linux") else {
            return list;
        };
        let mut nodes: Vec<_> = dir.flatten().map(|e| e.path()).collect();
        nodes.sort();
        for node in nodes {
            // Each camera exposes several nodes; index 0 is the capture one.
            if std::fs::read_to_string(node.join("index")).is_ok_and(|i| i.trim() != "0") {
                continue;
            }
            let Some(file) = node.file_name().and_then(|f| f.to_str()) else {
                continue;
            };
            let name = std::fs::read_to_string(node.join("name")).map(|n| n.trim().to_owned());
            list.push(Device {
                id: format!("/dev/{file}"),
                name: name.unwrap_or_else(|_| file.to_owned()),
            });
        }
        list
    }
    #[cfg(target_os = "macos")]
    {
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-f", "avfoundation", "-list_devices", "true", "-i", ""]);
        parse_avfoundation(&stderr_of(c), false)
    }
    #[cfg(target_os = "windows")]
    {
        let mut c = ffmpeg::command(ffmpeg);
        c.args(["-list_devices", "true", "-f", "dshow", "-i", "dummy"]);
        parse_dshow(&stderr_of(c), false)
    }
}

pub fn sources(ffmpeg: Option<&Path>) -> Sources {
    let portal = is_wayland();
    Sources {
        portal,
        // On Wayland the compositor's own picker chooses the screen or window.
        screens: if portal { vec![] } else { screens() },
        windows: if portal { vec![] } else { windows() },
        mics: ffmpeg.map(mics).unwrap_or_default(),
        webcams: ffmpeg.map(webcams).unwrap_or_default(),
        outputs: outputs(),
        apps: apps(),
    }
}

/// What the current session can tell us about the cursor:
/// `(cursor, clicks, notes)`.
pub fn input_capabilities() -> (&'static str, &'static str, Vec<String>) {
    #[cfg(target_os = "linux")]
    if is_wayland() {
        let metadata = crate::recorder::portal::cursor_metadata_available();
        let buttons = crate::recorder::evdev_input::available();
        let mut notes = Vec::new();
        if !metadata {
            notes.push(
                "This compositor does not report the cursor position separately, so the cursor is recorded \
                 into the video and cannot be smoothed or resized."
                    .to_owned(),
            );
        }
        if !buttons {
            notes.push(
                "Clicks are estimated from cursor movement. For exact clicks, add your user to the \"input\" \
                 group (sudo usermod -aG input $USER) and log in again."
                    .to_owned(),
            );
        }
        let cursor = if metadata { "metadata" } else { "embedded" };
        let clicks = match (buttons, metadata) {
            (true, _) => "exact",
            (false, true) => "inferred",
            (false, false) => "none",
        };
        return (cursor, clicks, notes);
    }
    ("metadata", "exact", vec![])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avfoundation_listing() {
        let text = "[AVFoundation indev @ 0x7f8] AVFoundation video devices:\n\
            [AVFoundation indev @ 0x7f8] [0] FaceTime HD Camera\n\
            [AVFoundation indev @ 0x7f8] [1] Capture screen 0\n\
            [AVFoundation indev @ 0x7f8] AVFoundation audio devices:\n\
            [AVFoundation indev @ 0x7f8] [0] MacBook Pro Microphone\n";
        let cams = parse_avfoundation(text, false);
        assert_eq!(cams.len(), 1);
        assert_eq!(
            (cams[0].id.as_str(), cams[0].name.as_str()),
            ("0", "FaceTime HD Camera")
        );
        assert_eq!(
            parse_avfoundation(text, true)[0].name,
            "MacBook Pro Microphone"
        );
    }

    #[test]
    fn dshow_listing() {
        let text = "[dshow @ 000] \"Integrated Camera\" (video)\n\
            [dshow @ 000]   Alternative name \"@device_pnp_x\"\n\
            [dshow @ 000] \"Microphone (Realtek Audio)\" (audio)\n";
        assert_eq!(parse_dshow(text, false)[0].id, "Integrated Camera");
        assert_eq!(parse_dshow(text, true)[0].id, "Microphone (Realtek Audio)");
        assert_eq!(parse_dshow(text, true).len(), 1);
    }

    #[test]
    fn pactl_listing() {
        let text = "Source #51\n\tState: SUSPENDED\n\tName: alsa_output.pci.analog-stereo.monitor\n\
            \tDescription: Monitor of Built-in Audio\nSource #52\n\tName: alsa_input.usb-Blue_Yeti\n\
            \tDescription: Blue Yeti Mono\n";
        let mics = parse_pactl(text, false);
        assert_eq!(mics.len(), 1);
        assert_eq!(
            (mics[0].id.as_str(), mics[0].name.as_str()),
            ("alsa_input.usb-Blue_Yeti", "Blue Yeti Mono")
        );
        let outs = parse_pactl(text, true);
        assert_eq!(
            (outs[0].id.as_str(), outs[0].name.as_str()),
            ("alsa_output.pci.analog-stereo.monitor", "Built-in Audio")
        );
    }

    #[test]
    fn pw_dump_lists_app_streams() {
        let text = r#"[
            {"id": 30, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Audio/Sink", "node.name": "alsa_output", "object.serial": 41}}},
            {"id": 108, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Stream/Output/Audio", "application.name": "Firefox",
                "media.name": "Big Buck Bunny", "object.serial": 2636}}},
            {"id": 109, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Stream/Output/Audio", "application.name": "mpv",
                "media.name": "mpv", "object.serial": "2700"}}},
            {"id": 110, "type": "PipeWire:Interface:Node", "info": {"props": {
                "media.class": "Stream/Input/Audio", "application.name": "Recorder"}}}
        ]"#;
        let apps = parse_pw_dump(text);
        let got: Vec<(&str, &str)> = apps
            .iter()
            .map(|d| (d.id.as_str(), d.name.as_str()))
            .collect();
        assert_eq!(got, [("2636", "Firefox — Big Buck Bunny"), ("2700", "mpv")]);
        assert!(parse_pw_dump("not json").is_empty());
    }
}
