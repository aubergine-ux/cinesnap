# Scripted demos

A scripted demo is a small YAML file that says what to do in a web app: open
this page, click that button, type here. `cinesnap-script` plays it in a
headless browser, records it as a normal [`.cinesnap` recording](recording-format.md),
and Cinesnap renders it like any other: zoom on the clicks, a smooth cursor,
a tidy frame.

Because nobody has to sit in front of a screen, the video can be regenerated
whenever the UI changes, for example [on every release](#keeping-a-readme-video-up-to-date-ci).

```yaml
# demo.yml
name: Quick tour
baseUrl: http://localhost:3000
serve: { command: "npm run dev", url: http://localhost:3000 }
steps:
  - open: /
  - wait: 800
  - click: "text=Get started"
  - type: { into: "#email", text: "ada@example.com" }
  - press: Enter
  - waitFor: ".toast"
  - wait: 1500
```

```sh
cinesnap-script run demo.yml --render demo.gif --preset readme
```

## Running locally

You need Node 20 or newer, `ffmpeg`, and the Cinesnap CLI (`cinesnap-cli`, or
the desktop app's `cinesnap` binary) for zooms and rendering.

```sh
cd scripted
npm install
npx playwright install chromium     # once; or see CINESNAP_SCRIPT_CHROMIUM below

# the bundled example: a small static site and a script that drives it
node bin/cinesnap-script.mjs run ../examples/demo.yml --render /tmp/demo.gif
```

```
cinesnap-script run <script.yml> [-o demo.cinesnap] [--render out.gif] [options]
cinesnap-script check <script.yml>
```

| Command / option | |
| --- | --- |
| `run <script>` | Record the script. Without `-o` or `--render` the bundle is written to `<script name>.cinesnap` in the current directory. |
| `check <script>` | Validate the script without opening a browser. Every problem is listed with its step number. |
| `-o, --out <dir>` | Where to write the `.cinesnap` bundle. Open it in the Cinesnap editor to tweak zooms by hand. |
| `--render <file>` | Also render the recording with the Cinesnap CLI. The extension picks the format (`.gif` or `.mp4`). The exit status is the renderer's. |
| `--preset <name>` | Render preset, passed straight to the Cinesnap CLI (`readme` is a GIF sized for a GitHub README). |
| `--keep` | With `--render` and no `-o`, the bundle is temporary and deleted afterwards; this keeps it and prints where it is. |
| `--headed` | Show the browser window while recording. Handy when a step cannot find its element. |
| `--timeout <ms>` | How long a step waits for its element (default 15000). Overrides `timeout` in the script. |
| `--cinesnap <path>` | The `cinesnap-cli` or `cinesnap` binary. Default: `$CINESNAP_BIN`, then `cinesnap-cli`, then `cinesnap` on `PATH`. |
| `--ffmpeg <path>` | The ffmpeg binary. Default: `$CINESNAP_FFMPEG`, then `PATH`. |

Environment:

| Variable | |
| --- | --- |
| `CINESNAP_BIN` | Path to the Cinesnap CLI. |
| `CINESNAP_FFMPEG` | Path to ffmpeg (the Cinesnap CLI honours the same variable). |
| `CINESNAP_SCRIPT_CHROMIUM` | Record with this Chromium or Chrome binary instead of the one Playwright downloads, for example `/usr/bin/chromium`. |

Without the Cinesnap CLI, `run` still writes a valid bundle, but with no zoom
segments (it prints a warning). `cinesnap-cli autozoom <bundle>` adds them
later.

## Script reference

The file is YAML; JSON works too. Unknown options are errors, so typos do not
pass silently.

### Top level

| Key | Default | |
| --- | --- | --- |
| `name` | `Demo` | A label for the script. |
| `viewport` | `{ width: 1280, height: 720 }` | Page size in CSS pixels. Add `deviceScaleFactor: 2` for a crisp 2x recording (the video is then 2560x1440). |
| `fps` | `30` | Frame rate of the recorded video, 1 to 60. |
| `colorScheme` | browser default | `light`, `dark` or `no-preference`: what `prefers-color-scheme` reports. |
| `baseUrl` | | `open` paths are resolved against this. |
| `serve` | | Start the app before recording; see below. |
| `style` | | Overrides for the look of the render; see below. |
| `pointer` | `{ speed: 1 }` | `speed` scales how fast the cursor travels (2 = twice as fast). `start: { x, y }` is where the cursor sits when the video begins (default: centre, a little below the middle). |
| `timeout` | `15000` | Milliseconds a step waits for its element before failing. |
| `seed` | `1` | Seed for the small random variations in cursor paths and typing rhythm. The same seed gives the same motion. |
| `steps` | required | The actions, in order. The first one must be `open`. |

All durations are milliseconds, except `serve.timeout`, which is seconds.

#### `serve`

Either run a command and wait for it:

```yaml
serve:
  command: npm run dev          # run through the shell
  url: http://localhost:3000    # recording starts once this answers
  timeout: 60                   # seconds to wait (default 60)
  cwd: .                        # where to run it, relative to the script (default: the script's directory)
```

or serve a directory of static files with the built-in server:

```yaml
serve:
  dir: ./site                   # relative to the script
  port: 0                       # default 0: any free port
```

When no `baseUrl` is set, the served address becomes the `baseUrl`. The
command and everything it started are stopped when
the recording ends, whether it succeeded or not. If `url` already answers
before the command starts, the run stops rather than record some other server.

#### `style`

Merged into the recording's edit state, so the render looks the way you want
without opening the editor. The sections are the ones in
[`project.json`](recording-format.md#projectjson): `frame`, `cursor`,
`autoZoom` and `export`.

```yaml
style:
  frame:
    aspect: "16:9"
    padding: 0.06
    background: { type: gradient, from: "#0f172a", to: "#334155", angle: 135 }
  cursor: { size: 1.8, smoothing: 0.7 }
  autoZoom: { scale: 1.8 }      # or { enabled: false } for no zooms
```

### Steps

Each step is a mapping with exactly one action. Most take a short form (just
the main value) or a long form with options. Selectors are
[Playwright selectors](https://playwright.dev/docs/locators): CSS, `text=…`,
`role=button[name="Save"]`, `>> nth=1` and so on. A selector has to match
exactly one element.

Steps that point at an element wait for it to be visible and to stop moving,
scroll it into view if needed (with a visible, smooth scroll), then glide the
cursor to it. Every such step accepts `timeout` (ms).

#### `open`

Go to a page and wait for it to load.

```yaml
- open: /pricing
- open: { url: "https://example.com/app", waitUntil: networkidle, timeout: 30000 }
```

`waitUntil` is `load` (default), `domcontentloaded`, `networkidle` or
`commit`. A full URL is used as is; anything else is resolved against
`baseUrl`. Without a `baseUrl`, it is a file next to the script
(`open: site/index.html`).

The first `open` happens before the recording starts, so the video begins on
the loaded page. Later ones are recorded like everything else.

#### `wait` / `pause`

Do nothing for a while. The two names mean the same.

```yaml
- wait: 800
- pause: 500
```

Give viewers time: a second after something appears, a little longer at the
end.

#### `click`

Move to an element and click its centre.

```yaml
- click: "text=Get started"
- click:
    selector: ".row >> nth=2"
    button: right              # left (default), right, middle
    count: 2                   # 2 = double click
    position: { x: 12, y: 8 }  # offset from the element's top-left instead of its centre
    hold: 90                   # ms the button stays down
```

#### `type`

Type text, one key at a time.

```yaml
- type: "hello"                                        # into whatever has focus
- type: { into: "#email", text: "ada@example.com", delay: 60 }
```

With `into`, the field is clicked first. `delay` is the average time between
keys (default 60); the actual gaps vary a little. A newline in the text
presses Enter, a tab presses Tab.

#### `press`

Press a key or a shortcut.

```yaml
- press: Enter
- press: Control+K
- press: { key: ArrowDown, times: 3, delay: 150 }
```

Modifiers are `Control` (`Ctrl`), `Alt` (`Option`), `Shift`, `Meta` (`Cmd`,
`Super`, `Win`) and `ControlOrMeta` (`Mod`), which is Meta on macOS and
Control elsewhere. Keys are single characters, `F1`–`F12`, or names: `Enter`,
`Escape`, `Tab`, `Space`, `Backspace`, `Delete`, `ArrowUp`/`Up`,
`ArrowDown`/`Down`, `ArrowLeft`/`Left`, `ArrowRight`/`Right`, `Home`, `End`,
`PageUp`, `PageDown`, `Insert`.

#### `hover`

Move to an element and stay there.

```yaml
- hover: ".card >> nth=1"
- hover: { selector: "#chart", position: { x: 200, y: 80 } }
```

#### `move`

Move the cursor to a point (CSS pixels from the top-left of the page view) or
to an element, optionally over a set time.

```yaml
- move: { x: 640, y: 360 }
- move: { to: "#logo", duration: 900 }
- move: "#logo"
```

#### `scroll`

Scroll smoothly, with the wheel, at the cursor's position.

```yaml
- scroll: 600                      # down 600 px
- scroll: { y: -300 }              # back up
- scroll: { x: 400, duration: 800 }
- scroll: { to: "#pricing" }       # until this element is centred
```

The wheel scrolls whatever is under the cursor, like a real one: to scroll a
panel rather than the page, `hover` it first.

#### `waitFor`

Wait for an element, without moving the cursor.

```yaml
- waitFor: ".toast"
- waitFor: { selector: ".spinner", state: hidden, timeout: 20000 }
```

`state` is `visible` (default), `hidden`, `attached` or `detached`.

#### `select`

Pick an option in a `<select>`, by its value or its label.

```yaml
- select: { from: "#country", option: "Norway" }
```

The cursor clicks the control and the value changes. The browser's own
drop-down list is not part of the page, so it does not appear in the video.

#### `check`

Tick a checkbox or radio button; `checked: false` unticks. Nothing is clicked
if it is already in that state.

```yaml
- check: "#agree"
- check: { selector: "#newsletter", checked: false }
```

#### `drag`

Press on one element, move to another, release.

```yaml
- drag: { from: ".card >> nth=0", to: "#done-column" }
- drag: { from: "#handle", to: "#target", hold: 200 }
```

`hold` is how long the button is held before the cursor sets off. This works
with mouse-event based drag and drop (most sortable lists and sliders).

## How it works

**The video.** Frames come from Chromium's screencast: every frame the page
presents, as a lossless image, stamped with the time it was shown. There is
no cursor in them, which is what Cinesnap wants; it draws its own. The frames
are laid onto a constant frame rate (each output frame shows the newest
capture as of the middle of its interval) and encoded as H.264. Time zero is
the first captured frame.

**The event log.** Every mouse move, button press, key and wheel step is
written to `events.jsonl` with the time it was sent to the page, on the same
clock as the frame stamps. Positions are in video pixels, so with
`deviceScaleFactor: 2` they are twice the CSS values. Measured on the
project's test page, a click shows up in the video within about one frame of
its logged time, which is how the redrawn cursor and click ripples line up
with what the page does.

**Cursor paths.** The cursor does not teleport. Each move is sampled 60 times
a second along a slightly bowed line, starting and stopping gently, and takes
longer the further it goes (roughly 0.45 s for 100 px, 0.9 s across the
page). Before a click the cursor rests for a moment. The page sees real mouse
events along the way, so hover states and tooltips behave as they would for a
person.

**Keys.** A `type` step logs that a key went down, once per character, and
nothing else: typed text never reaches the event log. A `press` step also
records the shortcut as a display string (`"combo":"Ctrl+K"`).

**Zooms and style.** With the bundle written, `cinesnap-cli autozoom` adds
zoom segments around the clicks and typing, exactly as for a hand-made
recording, and `--render` hands the bundle to `cinesnap-cli render`.

## Keeping a README video up to date (CI)

The repository is also a GitHub Action. It installs ffmpeg, the Cinesnap CLI
and the script runner, records your script, and commits the result if it
changed.

```yaml
# .github/workflows/demo.yml
name: Demo video

on:
  release:
    types: [published]
  workflow_dispatch:

permissions:
  contents: write        # to push the regenerated file

jobs:
  demo:
    runs-on: ubuntu-latest
    steps:
      # A release event checks out the tag; commit to the branch instead.
      - uses: actions/checkout@v4
        with:
          ref: ${{ github.event.repository.default_branch }}

      # Whatever your `serve.command` needs.
      - uses: actions/setup-node@v4
        with:
          node-version: 22
      - run: npm ci

      - uses: aubergine-ux/cinesnap@v0
        with:
          script: demo.yml
          output: docs/demo.gif
```

Then show it in the README:

```markdown
![Demo](docs/demo.gif)
```

| Input | Default | |
| --- | --- | --- |
| `script` | required | The demo script. |
| `output` | `docs/demo.gif` | File to render; `.gif` or `.mp4`. |
| `preset` | `readme` | Cinesnap render preset. `readme` is a GIF that fits GitHub's 10 MB limit for images. |
| `version` | `latest` | Cinesnap release to render with, for example `v0.2.0`. |
| `commit` | `true` | Commit the file to the checked-out branch when it changed. |
| `commit-message` | `docs: regenerate demo` | |
| `working-directory` | `.` | Where `script` and `output` are relative to. |

| Output | |
| --- | --- |
| `path` | The rendered file, relative to the workspace. |
| `changed` | `true` when the file differs from what is checked in. |

Notes:

- Linux runners (`ubuntu-latest`) are the supported ones. macOS runners
  should work; Windows is untested.
- Two recordings of the same script are never byte-identical, so every run
  produces a commit. Trigger it on releases or by hand rather than on every
  push.
- The commit is made as `github-actions[bot]` with the workflow's token.
  Pushes made that way do not start other workflows. A protected branch that
  rejects direct pushes will reject this one too; set `commit: false` and
  open a pull request with the file yourself (the `path` and `changed`
  outputs are there for that).
- When a release has no prebuilt CLI for the runner, the action builds it
  from source with `cargo install`, which takes several minutes.

## Limitations

- Browser only. The script drives a web page; native windows, the desktop,
  and anything outside the page (browser chrome, native menus, file dialogs,
  the list of a `<select>`) are not recorded.
- No audio: no microphone, no system sound, no webcam.
- It is headless Chromium. Fonts, scrollbars and form controls look the way
  they do there, which can differ from Chrome on your desktop, and from
  Firefox or Safari. On CI the fonts are the runner's; ship web fonts if the
  exact typeface matters.
- One page. Pop-up windows and new tabs are not followed.
- Frames are captured at up to about 60 a second. Pages that are heavy to
  render, large viewports and `deviceScaleFactor: 2` lower that, which shows
  as less smooth animation, not as lost sync.
- Selectors must match one element. Elements inside iframes are not
  supported.

## Troubleshooting

**`Chromium for Playwright is not installed`** — run
`npx playwright install chromium` in `scripted/`, or set
`CINESNAP_SCRIPT_CHROMIUM` to an installed Chromium or Chrome.

**`step 4 (click "…"): … Timeout 15000ms exceeded`** — the element never
became visible. Check the selector, add a `waitFor` on something that marks
the page as ready, or run with `--headed` and watch. `strict mode violation`
in the message means the selector matched several elements; narrow it or add
`>> nth=0`.

**A step times out on an element that is clearly there** — something is
covering it (a cookie banner, an overlay still fading out). Dismiss it in an
earlier step or wait for it to go: `waitFor: { selector: ".overlay", state: hidden }`.

**`serve: http://… did not respond`** — the command's last output is printed
below the error. Check the port and raise `serve.timeout` for slow builds.

**`serve: … already responds`** — another copy of the app is running on that
port. Stop it, so the recording is of the code you think it is.

**The cursor or click ripple is slightly off the element** — the element
moved after the cursor arrived (late layout shift, an animation that starts
on hover). Add a short `wait` before the step.

**The video starts on a half-rendered page** — `open` waits for the load
event only. Use `waitUntil: networkidle`, or follow it with a `waitFor`.

**The zoom goes somewhere odd while typing** — zooms follow the cursor. Put
it on the field first: `type` with `into` does that, or `hover` the field.

**Text looks soft** — set `viewport.deviceScaleFactor: 2`.

**`ffmpeg not found`** — install it, or pass `--ffmpeg` / set
`CINESNAP_FFMPEG`.
