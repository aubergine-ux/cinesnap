<div align="center">

<a href="https://aubergine-ux.github.io/cinesnap/">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/media/banner-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="docs/media/banner-light.png">
    <img src="docs/media/banner-light.png" alt="Cinesnap" width="420">
  </picture>
</a>

### Record your screen. Get a polished demo.

[![CI](https://img.shields.io/github/actions/workflow/status/aubergine-ux/cinesnap/ci.yml?branch=main&style=flat-square&label=build&logo=githubactions&logoColor=white)](https://github.com/aubergine-ux/cinesnap/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/aubergine-ux/cinesnap?style=flat-square&color=6d5dfc&label=release)](https://github.com/aubergine-ux/cinesnap/releases/latest)
[![License](https://img.shields.io/github/license/aubergine-ux/cinesnap?style=flat-square&color=8b7dff)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-backend-e44d8a?style=flat-square&logo=rust&logoColor=white)](crates/cinesnap-core)
[![TypeScript](https://img.shields.io/badge/TypeScript-frontend-3178c6?style=flat-square&logo=typescript&logoColor=white)](src)
[![Platforms](https://img.shields.io/badge/Windows%20%C2%B7%20macOS%20%C2%B7%20Linux-supported-2ea44f?style=flat-square)](#platform-support)

<a href="docs/media/before-after.mp4">
  <img src="docs/media/before-after.gif" alt="The same recording twice: the raw capture on the left, the Cinesnap export on the right" width="100%">
</a>

<sub>The same 12 seconds of screen activity. <b>Left:</b> the raw capture. <b>Right:</b> what Cinesnap exports, with no manual editing. <a href="docs/media/before-after.mp4">Watch as video</a></sub>

<br>

[**Download**](https://github.com/aubergine-ux/cinesnap/releases/latest) &nbsp;·&nbsp;
[Features](#features) &nbsp;·&nbsp;
[Install](#install) &nbsp;·&nbsp;
[Usage](#usage) &nbsp;·&nbsp;
[Platform support](#platform-support) &nbsp;·&nbsp;
[Docs](https://aubergine-ux.github.io/cinesnap/)

</div>

<br>

Good product demos zoom in on the thing being clicked, keep the cursor calm and easy to follow, and sit on a tidy background. Making one normally means recording, then an hour in a video editor adding keyframes.

Cinesnap records the screen **and** a log of where the cursor was and when you clicked or typed. The video is never touched; the effects are computed from the log. Press record, press export, and the result already looks edited. Everything stays adjustable afterwards.

## Features

<table>
  <tr><th colspan="3" align="left">Record</th></tr>
  <tr>
    <td align="center" width="56">🔍</td>
    <td width="170"><b>Auto-zoom</b></td>
    <td>Eases in on every click and typing burst and back out again. Clicks close together share one zoom that follows the cursor.</td>
  </tr>
  <tr>
    <td align="center">🖱️</td>
    <td><b>Cursor polish</b></td>
    <td>The system cursor is left out of the recording. A larger, smoothed one is drawn from the log, with click ripples, and still lands exactly on target.</td>
  </tr>
  <tr>
    <td align="center">🎙️</td>
    <td><b>Mic and webcam</b></td>
    <td>Recorded as separate tracks. The webcam bubble can be a circle, rounded or square, dragged anywhere, and moved, resized or hidden at any point on the timeline.</td>
  </tr>
  <tr><th colspan="3" align="left">Edit</th></tr>
  <tr>
    <td align="center">🎞️</td>
    <td><b>Clips and transitions</b></td>
    <td>Join several recordings into one video with cuts, crossfades or a zoom-through. Trim, cut out mistakes, reorder.</td>
  </tr>
  <tr>
    <td align="center">✏️</td>
    <td><b>Annotations</b></td>
    <td>Text callouts, arrows, boxes, highlights and spotlights that stay attached to the content when the camera zooms. Blur regions hide anything sensitive.</td>
  </tr>
  <tr>
    <td align="center">⌨️</td>
    <td><b>Keystrokes</b></td>
    <td>Shortcuts appear as keycaps in the video's own font and colours. Only shortcuts are recorded, never what you type.</td>
  </tr>
  <tr>
    <td align="center">🪧</td>
    <td><b>Title and outro cards</b></td>
    <td>Cards with your text, colours and fonts, anywhere on the timeline.</td>
  </tr>
  <tr>
    <td align="center">🎨</td>
    <td><b>Style presets</b></td>
    <td>Background, padding, cursor, fonts and colours saved as a preset, so every video looks the same. Four are built in.</td>
  </tr>
  <tr>
    <td align="center">↩️</td>
    <td><b>Undo and autosave</b></td>
    <td>Every edit can be undone, and every edit is on disk within half a second. A crash loses nothing.</td>
  </tr>
  <tr><th colspan="3" align="left">Sound</th></tr>
  <tr>
    <td align="center">💬</td>
    <td><b>Captions</b></td>
    <td>Transcribed on your machine with whisper.cpp, fully offline. Correct the text, pick an animated style, export <code>.srt</code> or <code>.vtt</code>.</td>
  </tr>
  <tr>
    <td align="center">✂️</td>
    <td><b>Edit by transcript</b></td>
    <td>Delete words in the transcript and the video is cut to match. One click finds every "um" and long silence, and shows them to you before cutting.</td>
  </tr>
  <tr>
    <td align="center">🎚️</td>
    <td><b>Voice cleanup</b></td>
    <td>Noise reduction, loudness normalisation and a simple voice EQ, or one <i>Enhance voice</i> button.</td>
  </tr>
  <tr>
    <td align="center">🎵</td>
    <td><b>Music</b></td>
    <td>Five built-in tracks (generated by Cinesnap, free to use anywhere) or your own file, lowered automatically while you speak.</td>
  </tr>
  <tr><th colspan="3" align="left">Ship</th></tr>
  <tr>
    <td align="center">📦</td>
    <td><b>Export presets</b></td>
    <td>GitHub README (a GIF or MP4 kept under 10 MB), X, YouTube, TikTok / Shorts, or plain 720p to 4K. Hardware encoding where the machine supports it.</td>
  </tr>
  <tr>
    <td align="center">🔗</td>
    <td><b>Share links</b></td>
    <td>An optional <a href="share-server">self-hosted share server</a>: upload from the app, get a link with a clean player page, view counts and an embed code.</td>
  </tr>
  <tr>
    <td align="center">🤖</td>
    <td><b>Scripted demos</b></td>
    <td>Describe a demo in a YAML file and Cinesnap plays it in a browser while recording. A <a href="docs/scripted-demos.md#keeping-a-readme-video-up-to-date-ci">GitHub Action</a> regenerates your README video on every release.</td>
  </tr>
  <tr>
    <td align="center">🖥️</td>
    <td><b>Command line</b></td>
    <td><code>cinesnap render demo.cinesnap --preset readme</code>, for scripts and CI.</td>
  </tr>
</table>

<p align="center">
  <img src="docs/media/editor.png" alt="The Cinesnap editor: preview with captions and a spotlight, timeline with clips, notes and zooms, transcript panel" width="92%">
</p>

## Install

Download the installer for your platform from the [**latest release**](https://github.com/aubergine-ux/cinesnap/releases/latest):

| Platform | File |
| :-- | :-- |
| 🪟 Windows 10/11 | `Cinesnap_x.y.z_x64-setup.exe` or `.msi` |
| 🍎 macOS 11+, Apple silicon | `Cinesnap_x.y.z_aarch64.dmg` |
| 🍎 macOS 11+, Intel | `Cinesnap_x.y.z_x64.dmg` |
| 🐧 Linux | `.AppImage`, `.deb` or `.rpm` |

Cinesnap needs **ffmpeg**. If it is not already installed, the start screen offers to download it into Cinesnap's own data folder.

<details>
<summary><b>Build from source</b></summary>

<br>

You need [Rust](https://rustup.rs), Node 20.19+ (or 22.12+), ffmpeg, CMake and a C++ compiler (speech recognition is compiled in), and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

On Linux, also install the PipeWire development files and clang:

```sh
# Debian / Ubuntu
sudo apt install libpipewire-0.3-dev libclang-dev cmake g++

# Fedora
sudo dnf install pipewire-devel clang-devel cmake gcc-c++

# Arch
sudo pacman -S pipewire clang cmake
```

Then:

```sh
git clone https://github.com/aubergine-ux/cinesnap
cd cinesnap
npm install
npm run tauri dev      # run the app
npm run tauri build    # build installers into target/release/bundle
```

The AppImage step downloads `linuxdeploy` and is only reliable on Debian and Ubuntu. Elsewhere, build the packages you need:

```sh
npm run tauri build -- --bundles deb,rpm
npm run tauri build -- --no-bundle     # just the binary: target/release/cinesnap
```

Run the tests:

```sh
cargo test --workspace
npm test
(cd share-server && npm test)
(cd scripted && npm ci && npm test)
```

To build without speech recognition (no CMake or C++ needed), turn the feature off: `cargo build -p cinesnap-core --no-default-features`.

</details>

## Usage

1. **Record.** Choose a screen or window (and a mic or webcam if you like) and press *Start recording*. Cinesnap gets out of the way.
2. **Stop.** Bring the window back and press *Stop*. The seconds you spent reaching for the button are trimmed for you.
3. **Finish it.** The editor opens with zooms already in place. Add what the video needs, in any order:
   - **Video** tab: more recordings, title and outro cards, transitions, crop, webcam.
   - **Timeline** toolbar: text, arrows, boxes, highlights, spotlights, blur.
   - **Captions** tab: generate captions, fix words, cut by deleting them, remove fillers and silences.
   - **Audio** tab: enhance the voice, add music.
   - **Style** tab: pick or save a look.
4. **Export.** Choose where it is going on the **Export** tab. With a share server set up, upload from the same dialog and paste the link.

<p align="center">
  <img src="docs/media/demo.gif" alt="A Cinesnap export: zooming to a click, following the cursor while typing" width="70%">
</p>

<details>
<summary><b>Captions, transcript editing and sound</b></summary>

<br>

- **Generate captions** downloads a speech model once (78 to 488 MB depending on the one you pick) and transcribes on your computer. Nothing is uploaded, and it works offline from then on.
- Click a word to jump to it. <kbd>Shift</kbd>-click selects a range; <kbd>Delete</kbd> cuts those words out of the video. Double-click a word to correct it.
- **Remove fillers & silences** lists every "um", "uh" and pause longer than you choose, each with a play button. Untick the ones to keep, then apply. One undo brings everything back.
- Caption styles: *Highlight word* (the spoken word lights up), *Word by word*, or *Plain*. Export `.srt` / `.vtt` for players that show their own subtitles.
- **Enhance voice** turns on noise reduction, loudness normalisation (-16 LUFS) and a voice EQ. Each can be adjusted on its own.
- Music loops for the length of the video, fades in and out, and ducks under speech. The soundtrack you hear in the editor is the one that gets exported.

</details>

<details>
<summary><b>Share server</b></summary>

<br>

A small server you host yourself. Upload a video, get a link with a player page, view counts and an embed code.

```sh
cd share-server
cp .env.example .env          # set UPLOAD_TOKEN, e.g. openssl rand -hex 24
docker compose up -d          # http://localhost:8080
```

Enter the address and token under **Export → Share server** in the app, or upload from a script:

```sh
cinesnap share demo.mp4 --server https://share.example.com --token "$TOKEN"
```

Reverse proxy setup, the HTTP API and backups are covered in [share-server/README.md](share-server/README.md).

</details>

<details>
<summary><b>Scripted demos and the GitHub Action</b></summary>

<br>

Describe the demo once and regenerate the video whenever the UI changes:

```yaml
# demo.yml
viewport: { width: 1280, height: 720 }
baseUrl: http://localhost:3000
steps:
  - open: /
  - click: "text=Get started"
  - type: { into: "#email", text: "ada@example.com" }
  - press: Control+K
  - wait: 800
```

```sh
cd scripted && npm ci
node bin/cinesnap-script.mjs run ../examples/demo.yml --render demo.gif --preset readme
```

In CI, on every release:

```yaml
- uses: aubergine-ux/cinesnap@main
  with:
    script: demo.yml
    output: docs/demo.gif
    preset: readme
```

Every step type, timing details and a complete workflow are in [docs/scripted-demos.md](docs/scripted-demos.md).

</details>

<details>
<summary><b>Keyboard shortcuts</b></summary>

<br>

| Key | Action |
| :-- | :-- |
| <kbd>Space</kbd> | Play / pause |
| <kbd>←</kbd> <kbd>→</kbd> | Step one frame (<kbd>Shift</kbd> for ten) |
| <kbd>S</kbd> | Mark a cut start, press again to cut to the playhead |
| <kbd>C</kbd> | Drag-to-cut mode |
| <kbd>Delete</kbd> | Remove the selection: a zoom, cut, annotation, clip, webcam keyframe, or the selected words |
| <kbd>Ctrl</kbd> <kbd>Z</kbd> / <kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>Z</kbd> | Undo / redo |
| <kbd>Ctrl</kbd> + scroll | Zoom the timeline |
| <kbd>Esc</kbd> | Clear the selection, leave cut mode |

</details>

<details>
<summary><b>Command line</b></summary>

<br>

The app binary doubles as a command-line tool. Recordings are plain folders (`*.cinesnap`) in `Videos/Cinesnap`.

```sh
# a README-sized GIF under GitHub's 10 MB limit, named after the project
cinesnap render "My demo.cinesnap" --preset readme

# other destinations
cinesnap render "My demo.cinesnap" --preset youtube -o demo.mp4
cinesnap render "My demo.cinesnap" --preset shorts  -o short.mp4

# with the settings saved in the recording
cinesnap render "My demo.cinesnap" -o demo.mp4

# join recordings, transcribe, write subtitles
cinesnap add "My demo.cinesnap" "Part two.cinesnap" --transition crossfade
cinesnap transcribe "My demo.cinesnap" --model base.en
cinesnap captions "My demo.cinesnap" -o demo.srt

# upload to your share server
cinesnap share demo.mp4 --server https://share.example.com --token "$TOKEN"

# open a recording straight in the editor
cinesnap "My demo.cinesnap"

# record five seconds without opening the window
cinesnap record out.cinesnap --seconds 5

# what this machine supports: capture, cursor, clicks, encoder, devices
cinesnap doctor
```

| `--preset` | Output |
| :-- | :-- |
| `readme` | GIF, 800 px wide, 15 fps, kept under 10 MB |
| `readme-mp4` | MP4, 720p, 30 fps, kept under 10 MB |
| `x` | MP4, 1080p, 30 fps |
| `youtube` | MP4, 1440p, 60 fps |
| `shorts` (or `tiktok`) | MP4, vertical 1080×1920, 30 fps |
| `720p` `1080p` `1440p` `4k` | MP4 at that size |
| `small` | GIF, 480 px wide |

| Option | Values |
| :-- | :-- |
| `--quality` | `low`, `medium`, `high` |
| `--aspect` | `16:9`, `9:16`, `1:1`, `4:3`, any other `W:H`, or `source` |
| `--fps` | 1 to 120, for presets that do not set their own |
| `--software` | Skip the hardware encoder |

For a machine without a desktop, `cargo build --release -p cinesnap-core` builds the same command line as `cinesnap-cli`, with no GUI dependencies. Releases include it as a standalone binary.

</details>

## How it works

```
 record                          edit                          export
┌───────────────┐   ┌──────────────────────────────┐   ┌─────────────────────┐
│ video.mp4     │   │ project.json                 │   │ ffmpeg decode       │
│ (no cursor)   │──▶│  clips · zooms · cuts ·      │──▶│  → compositor (Rust)│──▶ MP4 / GIF
│ events.jsonl  │   │  annotations · captions ·    │   │  → ffmpeg encode    │
│ mic · webcam  │   │  sound · style               │   │  + mixed soundtrack │
│               │   │ live canvas preview          │   └─────────────────────┘
└───────────────┘   └──────────────────────────────┘
```

- A recording is a folder holding the untouched video, an [event log](docs/recording-format.md) of cursor movement, clicks and key activity, and a `project.json` with every edit.
- One motion engine, written in Rust, turns the log into zoom segments, a camera path and a smoothed cursor path. The live preview and the exporter both read its output, so what you see is what you get.
- What you type is never logged, only that a key went down. Shortcuts (a key with Ctrl, Alt or the OS key) and keys like Enter are labelled for the keystroke overlay, and that can be turned off before recording.
- Speech recognition, audio cleanup and rendering all run on your machine. The only downloads are ffmpeg and a speech model, once each, and only if you use them.

More in [docs/architecture.md](docs/architecture.md).

## Platform support

| | Capture | Cursor position | Clicks and typing |
| :-- | :-- | :-- | :-- |
| **Windows 10/11** | Desktop Duplication | ✅ | ✅ |
| **macOS 11+** | AVFoundation | ✅ | ✅ |
| **Linux · X11** | x11grab | ✅ | ✅ |
| **Linux · Wayland** (KDE, GNOME) | ScreenCast portal + PipeWire | ✅ from stream metadata | ✅ with `input` group access, otherwise estimated |
| **Linux · Wayland** (portals without cursor metadata) | ScreenCast portal + PipeWire | ⚠️ cursor stays in the video | ⚠️ timing only |

<details>
<summary><b>Wayland and the <code>input</code> group</b></summary>

<br>

Wayland deliberately hides global input from applications. Cinesnap reads the cursor position from the screen-sharing stream and button presses from the kernel input devices. For the latter, your user needs to be in the `input` group:

```sh
sudo usermod -aG input $USER   # then log out and back in
```

Without it, clicks are estimated from where the cursor comes to rest. The trade-offs are written up in [the architecture notes](docs/architecture.md#wayland-limitations-the-important-part).

</details>

> [!NOTE]
> Cinesnap is young. Day-to-day development happens on Linux; the Windows and macOS builds are compiled and unit-tested in CI but have had far less hands-on use. Bug reports from those platforms are especially welcome.

## Contributing

Issues and pull requests are welcome. The codebase is small and split along clear lines:

| Path | What lives there |
| :-- | :-- |
| [`crates/cinesnap-core`](crates/cinesnap-core) | Project format, event log, motion engine, compositor, export, command line. No GUI dependencies; most logic and tests are here. |
| [`src-tauri`](src-tauri) | The desktop app: capture backends, input tracking, commands. |
| [`src`](src) | The TypeScript/React frontend: preview renderer, timeline, settings. |
| [`share-server`](share-server) | The self-hosted share server (Node, no dependencies). |
| [`scripted`](scripted) | Scripted demos: a Playwright runner that writes ordinary recordings. |
| [`docs`](docs) | Architecture, file format, scripted demos, and the docs page. |

<br>

<div align="center">

<img src="docs/media/icon.png" width="44" alt="">

Released under the [MIT License](LICENSE).

Built with [Tauri](https://tauri.app), [ffmpeg](https://ffmpeg.org), [whisper.cpp](https://github.com/ggml-org/whisper.cpp), [tiny-skia](https://github.com/linebender/tiny-skia) and [PipeWire](https://pipewire.org).<br>
Bundled fonts (Inter, Poppins, PT Serif, IBM Plex Mono) are under the [SIL Open Font License](assets/fonts).

Made by [@aubergine-ux](https://github.com/aubergine-ux)

</div>
