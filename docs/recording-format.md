# Recording format

A recording is a directory with the extension `.cinesnap`:

```
my-demo.cinesnap/
├── project.json     manifest + edit state (the only file the editor rewrites)
├── events.jsonl     input event log (append-only while recording)
├── video.mp4        screen, cursor hidden, constant frame rate
├── mic.wav          optional microphone track
├── webcam.mp4       optional webcam track
└── cursors/         optional cursor bitmaps captured during recording
    └── 3.png
```

Everything except `project.json` is immutable after recording stops, so edits
are cheap, undoable, and never degrade the source.

## Time base

All times are **milliseconds from the first video frame**, as floating point
numbers. Each track stores its own `offsetMs` in the manifest (when its first
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
| `key` | — | A key went down. The key itself is **never** recorded. |
| `scroll` | `dx`, `dy`, `x`, `y` | Wheel steps. |
| `cursor` | `id`, `hx`, `hy`, `w`, `h` | Cursor shape changed to `cursors/<id>.png` with the given hotspot. Only when the platform provides bitmaps. |
| `visible` | `v` | Cursor left or re-entered the captured area. |

Readers must ignore unknown keys and unknown event kinds. Lines are written in
non-decreasing `t`. A truncated last line (crash during recording) is skipped.

## `project.json`

```jsonc
{
  "format": "cinesnap-project",
  "version": 1,
  "createdAt": "2026-11-12T03:55:00Z",
  "app": "0.1.0",

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

  "webcam": { "show": true, "corner": "bottom-right", "size": 0.18, "shape": "circle" },

  "export": { "format": "mp4", "preset": "1080p", "fps": 60, "quality": "high" }
}
```

Times in `cuts` and `zooms` are **source time**; the mapping to output time
(after cuts) is derived, so cutting a section never invalidates zoom segments.
Unknown fields are preserved on save so newer projects survive older builds.
