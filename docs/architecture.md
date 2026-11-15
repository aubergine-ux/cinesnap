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
drops to almost nothing. The recorder only needs "a thing that writes
`video.mp4` and reports when its first frame arrived", so a native
ScreenCaptureKit or Windows.Graphics.Capture backend can be added later
without touching anything else — the main things they would buy are per-window
capture that follows a moving window and HDR handling.

ffmpeg cannot speak the Wayland portal protocol, so that backend is native:
frames arrive from PipeWire and are piped into an ffmpeg encoder as raw video.

### Cursor and click tracking

| Platform | Cursor position | Clicks / keys |
| --- | --- | --- |
| Windows, macOS, X11 | polled global pointer (`device_query`) | polled button and key state (`device_query`) |
| Wayland | PipeWire `SPA_META_Cursor` from the ScreenCast stream | `evdev` (read-only), with a fallback |

Windows on these platforms are listed with `xcap` and recorded as a crop of
the screen they are on (so the window should stay put while recording).

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
   `input` group (or a udev rule). Cinesnap records *that* a button or key
   went down and when. Typed characters are never recorded; only shortcuts
   and action keys get a label (see [Keystrokes](#keystrokes-and-privacy)).
3. **No `evdev` access → inferred clicks.** A dwell detector marks a click
   candidate when the cursor decelerates to a stop and stays within a few
   pixels for 350 ms after a deliberate move. It is less precise than real
   click data and is labelled as inferred in the event log, so auto-zoom can
   weight it lower. The app shows a one-line hint on how to enable exact clicks.
4. **Capture always goes through the portal picker.** The user chooses the
   screen or window in the system dialog; Cinesnap cannot enumerate windows
   itself. The picker is shown for every recording so the source can always
   be changed.
5. **Touchpad taps** are synthesised into clicks by the compositor and never
   appear as button events on the kernel device, so tap-to-click is only seen
   through the inferred-click path. Physical clicks are exact.
6. **Regions** are not a portal concept. Cinesnap records the whole monitor and
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
│   recorder/  portal + pipewire | ffmpeg grab, mic, webcam     │
│   recorder/  input: pipewire cursor meta + evdev | polling    │
│   devices    screens, windows, microphones, webcams           │
│   project/   bundle format, event log, edit state             │
│   motion/    auto-zoom, camera path, cursor smoothing         │
│   render/    frame compositor (CPU, rayon)                    │
│   export/    ffmpeg decode → compositor → ffmpeg encode       │
└───────────────────────────────────────────────────────────────┘
```

### Recording

The microphone and webcam are also captured by ffmpeg (PulseAudio/PipeWire,
AVFoundation or DirectShow), each in its own process. Every capture process
runs a `showinfo` filter; the moment its first log line appears is taken as
that track's first sample, which is how tracks started by different processes
are aligned without a shared clock inside the files.

**System sound** is its own track, so it can be turned down or muted without
touching the voice. Linux records a PulseAudio/PipeWire monitor with ffmpeg;
Windows (WASAPI loopback) and macOS 14.6+ (Core Audio taps) go through
`cpal`. **One app's sound** comes from its own PipeWire stream on Linux and
from process loopback on Windows 10 2004+. Loopback sources deliver nothing
while nothing plays, so they are written through a gap-filling writer
(`recorder/tape.rs`) that inserts the silence from the capture instants.

**Variable frame rate.** With "skip unchanged frames" on, `mpdecimate` drops
frames identical to the one before, keeping at least four a second. It runs
after the conversion to the encoder's pixel format, which happens anyway, so
it costs about 0.1 ms per 4K frame instead of the 3 ms it costs on packed RGB.

**HDR screens** (Windows) are captured in 10 bit and converted with two 1D
lookup tables and a matrix (`cinesnap_core::tonemap`): PQ to linear light at
the screen's own SDR white level, BT.2020 to BT.709, linear to sRGB. SDR
content comes out within one 8-bit step of an SDR capture; brighter
highlights clip. It is about 2.5x cheaper than ffmpeg's `zscale`/`tonemap`
chain and needs no optional ffmpeg libraries.

**Sessions and segments.** A recording session is made of segments. Pausing
stops every capture process, so nothing is recorded while paused; resuming
starts a new segment (on Wayland the PipeWire stream stays open, so the
picker does not come back). Switching screens starts the new segment before
stopping the old one and cuts the overlap. When stopped, the segments become
clips joined by plain cuts, and the countdown, the switch overlaps and the
reach for the stop button become ordinary, undoable cuts.

**Control bar and hotkeys.** A small always-on-top window shows the time
and the controls. Windows (`WDA_EXCLUDEFROMCAPTURE`) and macOS (sharing type
none) leave it out of the recording; Linux cannot, so the bar goes to a
screen that is not being recorded, or is shown only for the countdown.
Global hotkeys (start/stop, pause, marker) use the global-shortcut plugin,
and the GlobalShortcuts portal on Wayland, where the desktop owns the keys.
Cinesnap's own hotkeys are left out of the keystroke overlay.

Video, audio, webcam and input are recorded as independent tracks into a
project bundle (see [recording-format.md](recording-format.md)). The video is
a visually lossless H.264 intermediate with the cursor hidden. Nothing is
"baked in": every effect is computed later from the event log.

### One motion model, two renderers

All motion logic lives in Rust (`motion/`):

- **Auto-zoom** turns the event log (and, once analysed, where the screen
  changes) into zoom segments.
- **Reframing** places the window of a reframed recording.
- **Camera path** turns zoom segments into a camera (centre + scale) per frame,
  with spring-like easing and cursor following inside a segment.
- **Cursor smoothing** turns raw samples into the drawn cursor path.

The frontend never reimplements this. After each edit it calls
`compute_motion(project)` and receives sampled camera and cursor tracks
(typically < 5 ms for a ten-minute recording), then draws them on a canvas over
the `<video>` element. Export uses the same tracks with a Rust compositor. The
two renderers only share *drawing* rules (rounded rect, shadow, background),
which are simple; the hard-to-get-right *motion* cannot drift apart.

### Effects

**Where things change on screen.** Each recording is decoded once, in the
background, at 240 pixels wide and ten frames a second; every step records
the share of the picture that changed and the box around it
(`activity.rs`, cached in `cache/`). The cursor is not in the video, so
what is left is content: text appearing, a terminal scrolling. Typing
zooms anchor on the small, local changes that follow key presses (the
text cursor) instead of the last click, and the camera follows them while
typing. The export runs the same analysis if it is missing, so it always
matches the preview.

**Reframing.** With "fill the shape", a vertical or square output of a wide
recording shows a window of the output's shape instead of the whole
recording with bars. The window follows the cursor with a dead zone and
heavy, lag-free smoothing; while the mouse rests it follows what changes
on screen. Manual frame segments pin it, easing in and out
(`motion/reframe.rs`). For the camera this is only a different resting
centre and view size, so zooms work inside a reframed video unchanged.

**Motion blur** averages what the shutter would see: the recording is
drawn at camera states spread over the shutter interval and averaged, and
the cursor is drawn as a fading trail. The number of samples follows how
far the picture moves in output pixels (none for a still frame, up to 12),
so it costs nothing when nothing moves. The canvas preview does the same
with `globalAlpha` (copy *k* at 1/(*k*+1)).

**Device frames** (browser and desktop windows, laptop, phone) are part of
the layout: their insets are taken from the padded space before the
recording is fitted, so a frame never overflows, and they are drawn as
vectors by both renderers from the same numbers (`device.rs`,
`src/render/device.ts`).

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

The export path is also exposed as a CLI (`cinesnap render <project> --preset readme`, or the GUI-free `cinesnap-cli` binary)
which is what the test suite and the README demo video use.

### The timeline

A project is a list of clips: recordings and cards. Recordings added later
are copied (hard-linked where possible) into `clips/<id>/` inside the bundle,
so a project stays one self-contained folder. Edit state lives in *timeline
time*, the clips laid end to end; the per-recording motion code never sees
more than one clip, because `Project::local` hands it a single-recording view
with the cuts and zooms that fall inside that clip shifted to its own time.

Transitions do not overlap clips. A blend is centred on the join; the clip
that is not playing is rendered frozen on its first or last frame and the two
finished frames are mixed (`blend_transition`). Output time therefore stays a
simple function of the cuts, and audio needs no special handling.

Cards, annotations, the keystroke overlay and captions are drawn by the same
two renderers as everything else. Text is the one place where a browser and
a rasteriser disagree by default, so both sides lay text out the same
deliberately plain way: bundled fonts only, glyphs placed by advance width
(no kerning or ligatures), and vertical positions that never depend on font
metrics. `text.rs` and `src/render/text.ts` are that agreement.

### Keystrokes and privacy

The keystroke overlay needs to know which shortcut was pressed, which is at
odds with never recording keys. The rule (`events::shortcut`) is: a key gets
a label only if Ctrl, Alt or the OS key is held with it, or if it is an
action key (Enter, Esc, Tab, arrows, function keys…). Letters, digits and
punctuation typed on their own or with Shift are logged as "a key went down"
and nothing more, so typed text, passwords included, cannot be reconstructed
from a recording. Labelling can be switched off before recording.

### Sound

```
mic.wav ──ffmpeg: align · denoise · EQ · loudness──▶ cache/voice-<clip>.wav   (per recording, clip time)
                                                             │
music (built-in synth or a file) ──▶ cache/music-*.wav       ▼
                                          └──────▶ mixer (Rust): cuts · loop · duck ──▶ soundtrack.wav
```

Each microphone track is cleaned up once by ffmpeg and cached. The mixer then
walks the output timeline in Rust: it splices the voice at the cuts with 6 ms
fades (no clicks), loops the music, and lowers it under speech using a gain
envelope computed from the voice level, with a little lookahead so the music
is already down when a sentence starts. The export muxes that file; the
editor plays the very same file for the preview, rebuilt in the background
whenever an edit changes it.

The built-in music is synthesised by `music.rs`, so there are no audio assets
in the repository and nothing to license.

### Speech

Captions come from whisper.cpp, linked into the binary (`whisper` cargo
feature, on by default). The model file is downloaded once into the app's
data folder; after that nothing touches the network. Whisper is asked for one
word per segment, which gives word timings for free. Those timings drive the
karaoke captions, and they are what makes "delete a word, cut the video"
work: a transcript edit is just a cut over that word's time range, so it is
undoable like any other cut. Filler words come from the transcript; silences
come from the level of the cleaned-up voice track, relative to how loud the
speech is.

Long jobs (model download, transcription, soundtrack, upload, export) report
progress on one event channel and each has a cancel flag.

### Getting videos out

Export presets (`presets.rs`) carry a size rule, an optional frame rate and
aspect ratio, and an optional byte limit. For small limits (GitHub's 10 MB)
the frames are rendered once to a lossless intermediate and encoded as often
as it takes to fit: first at the chosen quality, then at whatever bitrate the
limit allows; GIFs step down in colours, size and frame rate.

The share server (`share-server/`) is a separate, dependency-free Node
program; the app and CLI upload to it with a bearer token. Scripted demos
(`scripted/`) drive a browser with Playwright and write an ordinary
recording bundle, so the rest of the pipeline does not know the difference.

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

React + Vite + TypeScript, state in a single store with undo/redo. Every edit
is a change to the project object, which is what makes undo cover everything
and autosave trivial: 400 ms after the last change the project is written
(atomically, with a backup), and again when the window is hidden or closed.
Layout: a large preview with handles for annotations and the webcam bubble, a
timeline docked at the bottom (clips, notes, webcam keyframes, zooms, cuts),
and a tabbed settings panel on the right. The preview is a 2D
canvas composited from the `<video>` element each animation frame.

## 5. Testing

- `motion/` and `project/` are pure and unit tested.
- The compositor and export pipeline are tested end to end against a synthetic
  recording (ffmpeg test pattern plus a generated event log).
- The project file is written by the frontend and read by the core, so a
  fixture that uses every editor feature is checked from both sides
  (`src/lib/contract.test.ts`, `tests/contract.rs`).
- The share server and the scripted-demo runner have their own test suites
  (`npm test` in each folder).
- Capture backends are thin by design. Linux backends are exercised manually;
  Windows and macOS are build-verified in CI.
