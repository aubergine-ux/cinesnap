# Cinesnap architecture

This document records the platform research behind Cinesnap, the decisions that
came out of it, and how the pieces fit together.

## 1. Platform research

### Screen capture

| Platform | Options considered | Decision |
| --- | --- | --- |
| Linux / Wayland | `xcap` (screenshot oriented, bare frame type), `scap` (stalled at 0.0.x, docs.rs build failing), `waycap-rs` (Wayland only), raw `ashpd` + `pipewire` | **`ashpd` ScreenCast portal + `pipewire` stream.** It is the only sanctioned path on Wayland, works on every portal-capable compositor, and is the only one that exposes cursor metadata. |
| Linux / X11 | `xcap`, XShm by hand, ffmpeg `x11grab` | **ffmpeg `x11grab`** with `-draw_mouse 0`. No extra native code; region and window capture are just geometry arguments. |
| Windows | `windows-capture` (Windows.Graphics.Capture), ffmpeg `ddagrab` / `gdigrab` | **ffmpeg `ddagrab`** (Desktop Duplication, `draw_mouse=0`) for screens and regions, `gdigrab` for single windows. |
| macOS | `screencapturekit` crate, ffmpeg `avfoundation` | **ffmpeg `avfoundation`** with `-capture_cursor 0`. Windows are captured as a crop of their display. |

Why ffmpeg does the grabbing on three of four backends: Cinesnap already needs
ffmpeg for encoding, so the capture → encode path becomes a single process with
no frame copies through Rust, and the amount of platform-specific unsafe code
drops to almost nothing. The capture layer is a trait (`CaptureBackend`), so a
native ScreenCaptureKit or Windows.Graphics.Capture backend can be added later
without touching anything else — the main things they would buy are per-window
capture that follows a moving window and HDR handling.

ffmpeg cannot speak the Wayland portal protocol, so that backend is native:
frames arrive from PipeWire and are piped into an ffmpeg encoder as raw video.

### Cursor and click tracking

| Platform | Cursor position | Clicks / keys |
| --- | --- | --- |
| Windows, macOS, X11 | polled global pointer (`device_query`) | polled button and key state (`device_query`) |
| Wayland | PipeWire `SPA_META_Cursor` from the ScreenCast stream | `evdev` (read-only), with a fallback |

Polling at 240 Hz was chosen over OS event hooks (`rdev`): no hook thread
constraints, no macOS main-thread crashes, no interference with the user's
input, and identical behaviour on three platforms. A press shorter than one
poll interval (~4 ms) can be missed; physical clicks are an order of magnitude
longer than that.

### Wayland limitations (the important part)

Wayland deliberately gives clients no global input. There is no protocol to ask
where the pointer is or to observe clicks outside your own surface, and the
`InputCapture` portal is for *taking over* input (Synergy-style), not for
observing it. What is available:

1. **Cursor position — ScreenCast cursor mode `Metadata`.** The compositor
   leaves the cursor out of the frames and attaches position, hotspot and the
   cursor bitmap to each PipeWire buffer. Coordinates are already in stream
   space, so no monitor-layout math is needed. KWin and Mutter support it;
   `xdg-desktop-portal-hyprland` and some wlroots portals do not. Cinesnap reads
   `AvailableCursorModes` and degrades as follows:
   - `Metadata` available → full cursor polish.
   - only `Hidden`/`Embedded` → record with the cursor embedded, disable cursor
     redraw, and derive zoom targets from clicks-without-position (zoom to
     centre) unless a compositor-specific source exists. The UI says so.
2. **Clicks and key activity — `evdev`.** Reading `/dev/input/event*` works
   below the compositor on any session type, but needs membership of the
   `input` group (or a udev rule). Cinesnap only records *that* a button or key
   went down and when; it never records which key.
3. **No `evdev` access → inferred clicks.** A dwell detector marks a click
   candidate when the cursor decelerates to a stop and stays within a few
   pixels for 350 ms after a deliberate move. It is less precise than real
   click data and is labelled as inferred in the event log, so auto-zoom can
   weight it lower. The app shows a one-line hint on how to enable exact clicks.
4. **Capture always goes through the portal picker.** The user chooses the
   screen or window in the system dialog; Cinesnap cannot enumerate windows
   itself. A restore token is stored so the dialog is skipped on later runs
   when the compositor allows it.
5. **Regions** are not a portal concept. Cinesnap records the whole monitor and
   stores the region as a crop in the project, which also makes the region
   adjustable after the fact. The same approach is used on every platform.

### Encoding

ffmpeg runs as a separate process (never linked), located in this order:
`CINESNAP_FFMPEG` → the copy Cinesnap downloaded into its data directory →
`PATH`. If none exists the app offers a one-click download. This keeps the
installer small and keeps GPL codecs out of the MIT-licensed binary.

Hardware encoders are detected by actually encoding a few test frames with each
candidate (`h264_nvenc`, `h264_videotoolbox`, `h264_qsv`, `h264_amf`,
`h264_vaapi`) — being listed in `ffmpeg -encoders` does not mean the driver
works. The first that succeeds is used; `libx264` is the fallback.

## 2. System overview

```
┌────────────────────────── Tauri app ──────────────────────────┐
│  Frontend (TypeScript, React, Canvas)                         │
│   Recorder UI · Preview renderer · Timeline · Settings panel  │
│            │ commands / events                                │
│  ──────────┼───────────────────────────────────────────────── │
│  Rust backend                                                 │
│   capture/   CaptureBackend: portal+pipewire | ffmpeg grab    │
│   input/     InputTracker:   pipewire meta + evdev | polling  │
│   audio/     microphone → WAV (cpal)                          │
│   project/   bundle format, event log, edit state             │
│   motion/    auto-zoom, camera path, cursor smoothing         │
│   render/    frame compositor (CPU, rayon)                    │
│   export/    ffmpeg decode → compositor → ffmpeg encode       │
└───────────────────────────────────────────────────────────────┘
```

### Recording

Video, audio, webcam and input are recorded as independent tracks into a
project bundle (see [recording-format.md](recording-format.md)). The video is
a visually lossless H.264 intermediate with the cursor hidden. Nothing is
"baked in": every effect is computed later from the event log.

### One motion model, two renderers

All motion logic lives in Rust (`motion/`):

- **Auto-zoom** turns the event log into zoom segments.
- **Camera path** turns zoom segments into a camera (centre + scale) per frame,
  with spring-like easing and cursor following inside a segment.
- **Cursor smoothing** turns raw samples into the drawn cursor path.

The frontend never reimplements this. After each edit it calls
`compute_motion(project)` and receives sampled camera and cursor tracks
(typically < 5 ms for a ten-minute recording), then draws them on a canvas over
the `<video>` element. Export uses the same tracks with a Rust compositor. The
two renderers only share *drawing* rules (rounded rect, shadow, background),
which are simple; the hard-to-get-right *motion* cannot drift apart.

### Export

```
ffmpeg (decode, kept ranges only) ──raw RGBA──▶ compositor ──raw RGBA──▶ ffmpeg (encode)
                                                   ▲
                             motion tracks, framing, cursor sprites, webcam frames
```

- **MP4**: H.264 via the detected hardware encoder or `libx264`, audio cut to
  the same kept ranges and muxed as AAC, `+faststart`.
- **GIF**: frames are composited at the GIF's size and frame rate, then encoded
  in two passes with a per-video palette (`palettegen` / `paletteuse` with
  rectangle diffing) for small, clean README GIFs.

The export path is also exposed as a CLI (`cinesnap render <project> -o out.mp4`)
which is what the test suite and the README demo video use.

## 3. Auto-zoom algorithm

1. Collect *interest points*: every button press, and every typing burst
   (key events less than 1.2 s apart, anchored at the last click before them).
2. Merge consecutive points into one segment when they are closer than 2.5 s
   in time; the segment then follows the cursor between them instead of zooming
   out and back in.
3. Each segment starts 0.35 s before its first point and holds 1.4 s after its
   last one, clamped so segments never overlap.
4. Segments shorter than the two easing ramps are stretched; inferred clicks
   need two points to form a segment.
5. Inside a segment the camera target is the smoothed cursor position, clamped
   so the view never leaves the recording, with a dead zone so small cursor
   movements do not move the camera.

Generated segments are ordinary editable data. Once the user touches a segment
it is marked `manual` and survives regeneration.

## 4. Frontend

React + Vite + TypeScript, state in a single store with undo/redo. Layout: a
large preview, a timeline docked at the bottom (zoom track, cut track,
playhead), and a collapsible settings panel on the right. The preview is a 2D
canvas composited from the `<video>` element each animation frame.

## 5. Testing

- `motion/` and `project/` are pure and unit tested.
- The compositor and export pipeline are tested end to end against a synthetic
  recording (ffmpeg test pattern plus a generated event log).
- Capture backends are thin by design. Linux backends are exercised manually;
  Windows and macOS are build-verified in CI.
