# Recording format

A recording is a directory with the extension `.cinesnap`:

```
my-demo.cinesnap/
├── project.json     manifest + edit state (the only file the editor rewrites)
├── events.jsonl     input event log (append-only while recording)
├── video.mp4        screen, cursor hidden, constant frame rate
├── mic.wav          optional microphone track
├── webcam.mp4       optional webcam track
├── cursors/         optional cursor bitmaps captured during recording
│   └── 3.png
├── clips/           recordings added to the project later, one folder each
│   └── clip1/       (same files as above: video.mp4, events.jsonl, …)
├── project.json.bak the manifest as it was before the last save
└── cache/           derived files: cleaned-up voice, music, preview mix
```

Everything except `project.json` is immutable after recording stops, so edits
are cheap, undoable, and never degrade the source. `cache/` can be deleted at
any time; it is rebuilt when needed.

Saving is crash-safe: the manifest is written to a temporary file, flushed to
disk and renamed over the old one, and the old one is kept as
`project.json.bak`. If `project.json` cannot be parsed, the backup is used.

## Time base

Inside a recording, all times are **milliseconds from the first video
frame**, as floating point numbers. In the manifest, edit state uses
**timeline time**: the clips laid end to end, before cuts (see
[Clips](#clips)). For a project with a single recording the two are the same. Each track stores its own `offsetMs` in the manifest (when its first
sample occurred on that clock), so tracks started by different processes stay
in sync. Events that happen before the first frame have negative times and are
kept; consumers ignore them.

## Coordinate space

All positions are in **source video pixels**, origin top-left, as floats. The
recorder converts from global/desktop coordinates at record time (subtracting
the capture origin and applying the display scale factor), so nothing
downstream needs to know about monitor layout or DPI. Positions may fall
outside `[0,width)×[0,height)` when the cursor leaves the captured area.

## `events.jsonl`

UTF-8 JSON Lines. The first line is a header; every following line is one
event. Keys are short because move events dominate the file.

```jsonl
{"format":"cinesnap-events","version":1,"width":2560,"height":1440,"clicks":"exact","cursor":"metadata"}
{"t":12.4,"k":"move","x":812.0,"y":430.5}
{"t":20.7,"k":"move","x":815.5,"y":431.0}
{"t":341.9,"k":"down","b":"left","x":903.0,"y":455.0}
{"t":422.3,"k":"up","b":"left","x":903.0,"y":455.0}
{"t":1310.0,"k":"key"}
{"t":1820.4,"k":"key","combo":"Ctrl+K"}
{"t":1402.2,"k":"scroll","dx":0,"dy":-3,"x":903.0,"y":455.0}
{"t":2051.6,"k":"cursor","id":3,"hx":4,"hy":2,"w":32,"h":32}
{"t":5000.1,"k":"visible","v":false}
```

### Header

| Field | Meaning |
| --- | --- |
| `format`, `version` | Always `"cinesnap-events"`; version is bumped on breaking changes. |
| `width`, `height` | Size of the coordinate space (the source video). |
| `clicks` | `"exact"` (OS or evdev), `"inferred"` (dwell detection), or `"none"`. |
| `cursor` | `"metadata"` (position known, cursor not in video), `"embedded"` (cursor is baked into the video; do not redraw), or `"none"`. |

### Events

| `k` | Fields | Notes |
| --- | --- | --- |
| `move` | `x`, `y` | Deduplicated; at most one per poll or frame. |
| `down` / `up` | `b`: `left` \| `right` \| `middle`, `x`, `y`, optional `inferred: true` | Inferred clicks have a `down` only. |
| `key` | optional `combo` | A key went down. Typed characters are **never** recorded. `combo` is present only for shortcuts (a key pressed with Ctrl, Alt or the OS key) and action keys (Enter, Esc, Tab, arrows, function keys…), as a display string such as `Ctrl+Shift+P`. It feeds the keystroke overlay and can be switched off when recording. |
| `scroll` | `dx`, `dy`, `x`, `y` | Wheel steps. |
| `cursor` | `id`, `hx`, `hy`, `w`, `h` | Cursor shape changed to `cursors/<id>.png` with the given hotspot. Only when the platform provides bitmaps. |
| `visible` | `v` | Cursor left or re-entered the captured area. |

Readers must ignore unknown keys and unknown event kinds. Lines are written in
non-decreasing `t`. A truncated last line (crash during recording) is skipped.

## `project.json`

```jsonc
{
  "format": "cinesnap-project",
  "version": 2,
  "createdAt": "2026-11-12T03:55:00Z",
  "app": "0.2.0",

  // What was recorded. Never changes after recording.
  "source": {
    "video":  { "file": "video.mp4", "width": 2560, "height": 1440, "fps": 60, "durationMs": 48211.0 },
    "mic":    { "file": "mic.wav", "offsetMs": -35.0 },        // optional
    "webcam": { "file": "webcam.mp4", "offsetMs": 120.0, "width": 1280, "height": 720 }, // optional
    "events": { "file": "events.jsonl" },
    "platform": "linux-wayland",
    "target": "screen"                                         // screen | window | region
  },

  // Everything below is edit state.
  "crop": { "x": 0, "y": 0, "width": 2560, "height": 1440 },   // region selection

  "cuts": [ { "startMs": 0, "endMs": 1200 } ],                 // removed ranges; trim = cut at either end

  "zooms": [
    {
      "id": "z1",
      "startMs": 2100, "endMs": 6400,       // source time
      "scale": 2.0,
      "easeInMs": 600, "easeOutMs": 700,
      "easing": "smooth",                   // smooth | snappy | linear
      "mode": "follow",                     // follow the cursor | fixed
      "x": 903.0, "y": 455.0,               // focus point (initial target, or the fixed one)
      "manual": false                       // true once edited or added by hand
    }
  ],

  "autoZoom": { "enabled": true, "scale": 2.0, "mergeGapMs": 2500, "onTyping": true },

  "cursor": {
    "show": true, "size": 1.6, "smoothing": 0.6,   // 0 = raw, 1 = very smooth
    "ripples": true, "rippleColor": "#ffffff",
    "hideWhenIdleMs": 0, "style": "arrow"
  },

  "frame": {
    "aspect": "16:9",                       // 16:9 | 9:16 | 1:1 | 4:3 | source
    "padding": 0.06,                        // fraction of the shorter output side
    "radius": 14, "shadow": 0.55,           // px at 1080p / 0..1
    "background": { "type": "gradient", "from": "#6d5dfc", "to": "#e44d8a", "angle": 135 }
    // or {"type":"solid","color":"#111"} | {"type":"image","path":"…"} | {"type":"wallpaper","path":"…","blur":40}
  },

  // corner is used until the bubble is placed freely (x, y: centre as fractions of the output)
  "webcam": { "show": true, "corner": "bottom-right", "x": 0.9, "y": 0.84, "size": 0.18, "shape": "circle" },  // circle | rounded | square
  "webcamKeys": [                                              // from atMs on, the bubble moves to this state
    { "id": "w1", "atMs": 6000, "x": 0.12, "y": 0.2, "size": 0.25, "show": true },
    { "id": "w2", "atMs": 15000, "x": 0.12, "y": 0.2, "size": 0.25, "show": false }
  ],

  "export": { "format": "mp4", "preset": "1080p", "fps": 60, "quality": "high" },

  // Everything on the timeline, in order. See "Clips" below.
  "clips": [
    { "id": "title", "kind": "card", "transition": { "type": "cut", "durationMs": 500 },
      "durationMs": 3000, "title": "Cinesnap", "subtitle": "…", "titleColor": "#ffffff",
      "subtitleColor": "#ffffffb3", "titleSize": 96, "align": "center" },   // optional: font, background
    { "id": "main", "kind": "recording", "transition": { "type": "crossfade", "durationMs": 500 },
      "dir": "", "source": { /* as above */ }, "crop": { "x": 0, "y": 0, "width": 2560, "height": 1440 } },
    { "id": "clip1", "kind": "recording", "transition": { "type": "zoom", "durationMs": 600 },
      "dir": "clips/clip1", "source": { /* that recording's tracks */ }, "crop": { /* … */ } }
  ],

  // Drawn over the recording between two timeline times. Positions are source
  // pixels of the clip on screen, so annotations follow the camera.
  "annotations": [
    { "id": "a1", "startMs": 2000, "endMs": 5000, "type": "text", "x": 700, "y": 450, "text": "Look here",
      "size": 44, "color": "#ffffff", "background": "#14151af0", "bold": true },        // optional: font
    { "id": "a2", "startMs": 2000, "endMs": 5000, "type": "arrow", "x1": 600, "y1": 640, "x2": 900, "y2": 480, "color": "#6d5dfc", "width": 8 },
    { "id": "a3", "startMs": 2000, "endMs": 5000, "type": "box", "x": 0, "y": 0, "width": 400, "height": 200, "color": "#6d5dfc", "stroke": 6, "radius": 8 },
    { "id": "a4", "startMs": 2000, "endMs": 5000, "type": "highlight", "x": 0, "y": 0, "width": 400, "height": 80, "color": "#ffe14a", "opacity": 0.7 },
    { "id": "a5", "startMs": 2000, "endMs": 5000, "type": "spotlight", "x": 0, "y": 0, "width": 400, "height": 200, "dim": 0.65, "ellipse": false },
    { "id": "a6", "startMs": 2000, "endMs": 5000, "type": "blur", "x": 0, "y": 0, "width": 400, "height": 200, "strength": 14 }
  ],

  // Fonts and colours shared by cards, callouts, captions and the keystroke overlay.
  "theme": { "font": "Inter", "accent": "#6d5dfc", "text": "#ffffff", "surface": "#14151af0" },
  "keystrokes": { "show": true, "position": "bottom", "size": 1.0, "holdMs": 1500 },

  "captions": { "show": true, "style": "karaoke", "y": 0.82, "size": 1.0, "color": "#ffffff",
                "highlight": "#ffd84a", "background": "#000000a6", "maxChars": 36, "uppercase": false },
  "transcript": { "language": "en", "words": [ { "t0": 560, "t1": 820, "text": "And" } ] },

  "audio": {
    "micVolume": 1.0, "denoise": 0.5, "normalize": true,
    "eq": { "enabled": true, "lowCutHz": 80, "bassDb": 0, "presenceDb": 3, "airDb": 1.5 },
    "music": { "source": "builtin:open-sky", "volume": 0.35, "duck": true, "duckDb": -14, "fadeInMs": 800, "fadeOutMs": 2500 }
  }
}
```

## Clips

`clips` is the timeline: recordings and cards, in order. A version 1 manifest
has no `clips`; readers treat it as a single recording clip made from the
top-level `source` and `crop` (which version 2 keeps for older builds).

**Timeline time** is the position in the clips laid end to end, each at its
full length. `cuts`, `zooms`, `annotations`, `webcamKeys` and the transcript
all use it. A zoom or annotation belongs to the clip it starts in; its
positions are in that recording's source pixels. Inserting, removing or
reordering clips shifts the times of everything attached to them.

A clip's `transition` describes how it comes in from the clip before it:
`cut`, `crossfade` or `zoom` (zoom-through). The blend is centred on the
join and neither clip is shortened: the clip that is not playing is shown on
its first (incoming) or last (outgoing) frame.

`export.preset` is one of `1080p`, `720p`, `1440p`, `4k` (MP4), `readme`,
`small` (GIF), or a destination preset: `readme-mp4`, `x`, `youtube`,
`shorts`. Destination presets may set their own frame rate and aspect ratio
and a file-size limit.

The mapping from timeline time to output time (after cuts) is derived, so
cutting a section never invalidates zoom segments, annotations or captions.
Unknown fields are preserved on save so newer projects survive older builds.
