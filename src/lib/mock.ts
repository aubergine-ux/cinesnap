// In-browser stand-in for the Rust backend, used when the app runs outside Tauri
// (plain `npm run dev`). It serves a demo project and synthetic motion tracks.

import { outputRatio, spans, totalDuration, webcamStateAt } from "./edit";
import { PRESETS, clampCentre, outputFps } from "./layout";
import { cutAt, keptRanges, normalizeCuts, outToSrc, srcToOut } from "./time";
import type {
  Capabilities,
  CaptionLine,
  Imported,
  MotionTracks,
  MusicTrack,
  Project,
  RecentProject,
  RecordingClip,
  RecordingOptions,
  Sources,
  Speech,
  Suggestion,
  TransitionMark,
  Word,
  ZoomSegment,
} from "./types";

type Listener = (payload: unknown) => void;
const listeners = new Map<string, Set<Listener>>();

export function mockListen(event: string, cb: Listener): () => void {
  if (!listeners.has(event)) listeners.set(event, new Set());
  listeners.get(event)!.add(cb);
  return () => listeners.get(event)?.delete(cb);
}

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload));
}

const DEMO_PATH = "mock://demo.cinesnap";
const W = 1920;
const H = 1080;
const DURATION = 24000;

// Scripted pointer: where the fake user clicks, and when.
const CLICKS = [
  { t: 2200, x: 420, y: 260 },
  { t: 3600, x: 560, y: 300 },
  { t: 8200, x: 1380, y: 640 },
  { t: 9500, x: 1460, y: 700 },
  { t: 10400, x: 1420, y: 760 },
  { t: 16000, x: 760, y: 820 },
  { t: 21000, x: 1240, y: 240 },
];
const KEYS = [4200, 4350, 4500, 4700, 4900, 5100, 5300, 16600, 16750, 16900, 17100, 17300];
const SHORTCUTS = [
  { t: 6500, combo: "Ctrl+K" },
  { t: 12500, combo: "Ctrl+Shift+P" },
  { t: 18200, combo: "Enter" },
];
const SCRIPT = "So this is the dashboard. Um, every card opens a detail view. Click one and it zooms in. Then, uh, type to search and press enter. That is the whole flow.";

/** The demo recording as the backend would hand it over. */
export function demoProject(): Project {
  const source: Project["source"] = {
    video: { file: "video.mp4", width: W, height: H, fps: 60, durationMs: DURATION },
    mic: { file: "mic.wav", offsetMs: 0 },
    events: { file: "events.jsonl" },
    platform: "mock",
    target: "screen",
  };
  const crop = { x: 0, y: 0, width: W, height: H };
  return {
    format: "cinesnap-project",
    version: 1,
    createdAt: new Date().toISOString(),
    app: "0.2.0",
    source,
    crop,
    cuts: [],
    zooms: [],
    autoZoom: { enabled: true, scale: 2, mergeGapMs: 2500, onTyping: true },
    cursor: { show: true, size: 1.6, smoothing: 0.6, ripples: true, rippleColor: "#ffffff", hideWhenIdleMs: 0, style: "arrow" },
    frame: {
      aspect: "16:9",
      padding: 0.06,
      radius: 14,
      shadow: 0.55,
      background: { type: "gradient", from: "#6d5dfc", to: "#e44d8a", angle: 135 },
    },
    webcam: { show: true, corner: "bottom-right", size: 0.18, shape: "circle" },
    export: { format: "mp4", preset: "1080p", fps: 60, quality: "high" },
    clips: [{ id: "main", kind: "recording", transition: { type: "cut", durationMs: 500 }, dir: "", source: structuredClone(source), crop: { ...crop } }],
    annotations: [],
    keystrokes: { show: true, position: "bottom", size: 1, holdMs: 1500 },
    webcamKeys: [],
    theme: { font: "Inter", accent: "#6d5dfc", text: "#ffffff", surface: "#14151af0" },
    captions: { show: true, style: "karaoke", y: 0.82, size: 1, color: "#ffffff", highlight: "#ffd84a", background: "#000000a6", maxChars: 36, uppercase: false },
    audio: { micVolume: 1, denoise: 0, normalize: false, eq: { enabled: false, lowCutHz: 80, bassDb: 0, presenceDb: 3, airDb: 1.5 }, music: undefined },
  };
}

/** A made-up transcript of the demo, as speech recognition would return it. */
function demoTranscript(): Word[] {
  let t = 1200;
  return SCRIPT.split(" ").map((text) => {
    const len = 140 + text.length * 45;
    const w = { t0: t, t1: t + len, text };
    t += len + (/[.,]$/.test(text) ? 520 : 70);
    return w;
  });
}

/** Group words into caption lines the way the backend does. */
function captionLines(p: Project): CaptionLine[] {
  const words = p.transcript?.words ?? [];
  const total = totalDuration(p);
  const cuts = normalizeCuts(p.cuts, total);
  const lines: CaptionLine[] = [];
  let chars = 0;
  for (const w of words) {
    if (cutAt(cuts, (w.t0 + w.t1) / 2) >= 0) continue;
    const startMs = srcToOut(w.t0, cuts, total);
    const endMs = Math.max(startMs + 1, srcToOut(w.t1, cuts, total));
    const last = lines[lines.length - 1];
    const prev = last?.words[last.words.length - 1];
    const fresh = !prev || chars + 1 + w.text.length > p.captions.maxChars || startMs - prev.endMs > 700 || (/[.?!]$/.test(prev.text) && chars * 2 >= p.captions.maxChars);
    if (fresh) {
      lines.push({ startMs, endMs, words: [] });
      chars = 0;
    } else chars += 1;
    chars += w.text.length;
    const line = lines[lines.length - 1];
    line.endMs = endMs;
    line.words.push({ text: w.text, startMs, endMs });
  }
  lines.forEach((l, i) => (l.endMs = Math.min(l.endMs + 350, lines[i + 1]?.startMs ?? Infinity)));
  return lines;
}

const projects = new Map<string, Project>();

function getProject(path: string): Project {
  if (!projects.has(path)) {
    const p = demoProject();
    p.zooms = autoZoom(p);
    projects.set(path, p);
  }
  return structuredClone(projects.get(path)!);
}

const smooth = (t: number) => {
  const x = Math.max(0, Math.min(1, t));
  return x * x * (3 - 2 * x);
};

function ease(kind: string, t: number): number {
  const x = Math.max(0, Math.min(1, t));
  if (kind === "linear") return x;
  if (kind === "snappy") return 1 - Math.pow(1 - x, 4);
  return smooth(x);
}

function cursorAt(ms: number): { x: number; y: number } {
  let prev = { t: 0, x: W / 2, y: H / 2 };
  for (const c of CLICKS) {
    if (ms <= c.t) {
      const travel = Math.min(900, c.t - prev.t);
      const k = smooth((ms - (c.t - travel)) / travel);
      return { x: prev.x + (c.x - prev.x) * k, y: prev.y + (c.y - prev.y) * k };
    }
    prev = c;
  }
  return { x: prev.x, y: prev.y };
}

function autoZoom(project: Project): ZoomSegment[] {
  const manual = project.zooms.filter((z) => z.manual);
  if (!project.autoZoom.enabled) return manual;
  // The scripted pointer only exists for the demo recording itself.
  const main = spans(project).find((s) => project.clips[s.index].id === "main");
  if (!main) return manual;
  return autoZoomMain(project, manual).map((z) => (z.manual ? z : { ...z, startMs: z.startMs + main.startMs, endMs: z.endMs + main.startMs }));
}

function autoZoomMain(project: Project, manual: ZoomSegment[]): ZoomSegment[] {
  const points = [...CLICKS.map((c) => c.t)].sort((a, b) => a - b);
  const groups: number[][] = [];
  for (const t of points) {
    const g = groups[groups.length - 1];
    if (g && t - g[g.length - 1] < project.autoZoom.mergeGapMs) g.push(t);
    else groups.push([t]);
  }
  const out: ZoomSegment[] = [];
  let prevEnd = 0;
  groups.forEach((g, i) => {
    let end = g[g.length - 1] + 1400;
    if (project.autoZoom.onTyping) {
      for (const k of KEYS) if (k > g[0] && k < end + 1200) end = Math.max(end, k + 1400);
    }
    const startMs = Math.max(prevEnd, g[0] - 350);
    const endMs = Math.min(DURATION, end);
    prevEnd = endMs;
    const first = CLICKS.find((c) => c.t === g[0])!;
    const seg: ZoomSegment = {
      id: `a${i}`,
      startMs,
      endMs,
      scale: project.autoZoom.scale,
      easeInMs: 600,
      easeOutMs: 700,
      easing: "smooth",
      mode: "follow",
      x: first.x,
      y: first.y,
      manual: false,
    };
    if (!manual.some((m) => m.startMs < seg.endMs && m.endMs > seg.startMs)) out.push(seg);
  });
  return [...manual, ...out].sort((a, b) => a.startMs - b.startMs);
}

function smoothMix(a: number, b: number, t: number): number {
  return a + (b - a) * smooth(t);
}

function computeMotion(project: Project): MotionTracks {
  const duration = totalDuration(project);
  const fps = outputFps(project.export);
  const cuts = normalizeCuts(project.cuts, duration);
  const outDuration = keptRanges(cuts, duration).reduce((s, r) => s + r.endMs - r.startMs, 0);
  const frames = Math.max(1, Math.floor((outDuration / 1000) * fps));
  const all = spans(project);
  const srcMs = new Array<number>(frames);
  const clip = new Array<number>(frames);
  const cam = new Array<number>(frames * 3);
  const cursor = new Array<number>(frames * 4);
  const webcam = new Array<number>(frames * 4);
  let at = 0;
  for (let i = 0; i < frames; i++) {
    const src = outToSrc((i * 1000) / fps, cuts, duration);
    while (at + 1 < all.length && src >= all[at].endMs) at++;
    srcMs[i] = src;
    clip[i] = at;
    const c = project.clips[at];
    const local = src - all[at].startMs;
    // Only the demo recording has a scripted pointer; other clips sit still.
    const scripted = c.kind === "recording" && c.id === "main";
    const crop = c.kind === "recording" ? c.crop : project.crop;
    const cx0 = crop.x + crop.width / 2;
    const cy0 = crop.y + crop.height / 2;
    const pos = scripted ? cursorAt(local) : { x: cx0, y: cy0 };
    let k = 0;
    let scale = 1;
    let tx = cx0;
    let ty = cy0;
    for (const z of project.zooms) {
      if (c.kind !== "recording" || src < z.startMs || src > z.endMs) continue;
      k = Math.min(ease(z.easing, (src - z.startMs) / Math.max(1, z.easeInMs)), ease(z.easing, (z.endMs - src) / Math.max(1, z.easeOutMs)));
      scale = 1 + (z.scale - 1) * k;
      tx = z.mode === "follow" ? pos.x : z.x;
      ty = z.mode === "follow" ? pos.y : z.y;
    }
    const centre = clampCentre(crop, cx0 + (tx - cx0) * k, cy0 + (ty - cy0) * k, scale);
    cam[i * 3] = centre.cx;
    cam[i * 3 + 1] = centre.cy;
    cam[i * 3 + 2] = scale;
    let press = 0;
    if (scripted) for (const cl of CLICKS) if (local >= cl.t && local < cl.t + 160) press = 1 - (local - cl.t) / 160;
    cursor[i * 4] = pos.x;
    cursor[i * 4 + 1] = pos.y;
    cursor[i * 4 + 2] = scripted ? 1 : 0;
    cursor[i * 4 + 3] = press;

    // Webcam bubble: ease from the previous keyframe state to this one.
    const keys = project.webcamKeys.filter((key) => key.atMs <= src);
    const to = webcamStateAt(project, src);
    const from = keys.length ? webcamStateAt(project, keys[keys.length - 1].atMs - 1) : to;
    const p = keys.length ? (src - keys[keys.length - 1].atMs) / 450 : 1;
    const hasCam = c.kind === "recording" && Boolean(c.source.webcam) && project.webcam.show;
    webcam[i * 4] = smoothMix(from.show ? from.x : to.x, to.show ? to.x : from.x, p);
    webcam[i * 4 + 1] = smoothMix(from.show ? from.y : to.y, to.show ? to.y : from.y, p);
    webcam[i * 4 + 2] = smoothMix(from.show ? from.size : to.size, to.show ? to.size : from.size, p);
    webcam[i * 4 + 3] = hasCam ? smoothMix(from.show ? 1 : 0, to.show ? 1 : 0, p) : 0;
  }

  const transitions: TransitionMark[] = [];
  let runStart = 0;
  let pending: TransitionMark | null = null;
  for (let i = 1; i <= frames; i++) {
    if (i < frames && clip[i] === clip[i - 1]) continue;
    const run = i - runStart;
    if (pending) {
      pending.half = Math.min(pending.half, Math.floor(run / 2));
      if (pending.half >= 1) transitions.push(pending);
      pending = null;
    }
    if (i < frames) {
      const tr = project.clips[clip[i]].transition;
      if (tr.type !== "cut") pending = { frame: i, half: Math.min(Math.round(((tr.durationMs / 2) * fps) / 1000), Math.floor(run / 2)), kind: tr.type };
      runStart = i;
    }
  }

  const main = all.find((s) => project.clips[s.index].id === "main");
  const inMain = <T extends { t: number }>(items: T[]) => (main ? items.map((it) => ({ ...it, t: it.t + main.startMs })) : []);
  const visible = (t: number) => cutAt(cuts, t) < 0;
  void outputRatio;
  return {
    fps,
    frames,
    durationMs: outDuration,
    srcMs,
    clip,
    clipStarts: all.map((s) => s.startMs),
    cam,
    cursor,
    webcam,
    ripples: inMain(CLICKS)
      .filter((c) => visible(c.t))
      .map((c) => ({ outMs: srcToOut(c.t, cuts, duration), x: c.x, y: c.y, clip: main!.index })),
    events: { clicks: inMain(CLICKS).map((c) => c.t), keys: inMain(KEYS.map((t) => ({ t }))).map((k) => k.t) },
    keys: inMain(SHORTCUTS)
      .filter((k) => visible(k.t))
      .map((k) => ({ outMs: srcToOut(k.t, cuts, duration), combo: k.combo })),
    captions: captionLines(project),
    transitions,
    cursorDrawn: project.clips.map((c) => c.kind === "recording" && project.cursor.show),
    cursorMode: "metadata",
  };
}

const MUSIC: MusicTrack[] = [
  { id: "open-sky", name: "Open Sky", mood: "Calm, spacious", bpm: 60, durationMs: 32000 },
  { id: "slow-coffee", name: "Slow Coffee", mood: "Mellow, laid-back", bpm: 80, durationMs: 48000 },
  { id: "bright-side", name: "Bright Side", mood: "Upbeat, light", bpm: 120, durationMs: 32000 },
  { id: "wooden-steps", name: "Wooden Steps", mood: "Warm, minimal", bpm: 96, durationMs: 40000 },
  { id: "deep-focus", name: "Deep Focus", mood: "Steady, focused", bpm: 100, durationMs: 38400 },
];

let styles: unknown[] = [];
let settings: Record<string, unknown> = {};
const cancelled = new Set<string>();

/** A fake long-running job that reports progress and can be cancelled. */
async function job(name: string, stage: string, steps = 12): Promise<void> {
  cancelled.delete(name);
  for (let i = 1; i <= steps; i++) {
    await wait(70);
    if (cancelled.has(name)) throw new Error("cancelled");
    emit("job://progress", { job: name, stage, fraction: i / steps });
  }
}

let recTimer: number | undefined;
let exportTimer: number | undefined;

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export async function mockInvoke(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
  switch (cmd) {
    case "get_capabilities":
      return {
        platform: "linux-wayland",
        cursor: "metadata",
        clicks: "exact",
        ffmpeg: { path: "/usr/bin/ffmpeg", version: "demo" },
        encoder: "libx264",
        notes: ["Demo mode: running in a browser without the desktop backend."],
      } satisfies Capabilities;
    case "download_ffmpeg":
      for (let i = 1; i <= 10; i++) {
        await wait(120);
        emit("ffmpeg://progress", { fraction: i / 10 });
      }
      return;
    case "list_sources":
      return {
        portal: false,
        screens: [
          { id: "0", name: "Built-in display", x: 0, y: 0, width: 2560, height: 1440 },
          { id: "1", name: "External display", x: 2560, y: 0, width: 1920, height: 1080 },
        ],
        windows: [
          { id: "w1", title: "Terminal" },
          { id: "w2", title: "Editor — cinesnap" },
        ],
        mics: [{ id: "default", name: "Default microphone" }],
        webcams: [{ id: "cam0", name: "Integrated camera" }],
      } satisfies Sources;
    case "start_recording": {
      void (args.options as RecordingOptions);
      const started = performance.now();
      window.clearInterval(recTimer);
      recTimer = window.setInterval(() => emit("recording://tick", { elapsedMs: performance.now() - started }), 100);
      return;
    }
    case "stop_recording":
      window.clearInterval(recTimer);
      return DEMO_PATH;
    case "cancel_recording":
      window.clearInterval(recTimer);
      return;
    case "list_recent":
      return [{ path: DEMO_PATH, name: "Demo recording", createdAt: new Date().toISOString(), durationMs: DURATION }] satisfies RecentProject[];
    case "open_project":
      return getProject(args.path as string);
    case "save_project":
      projects.set(args.path as string, structuredClone(args.project as Project));
      return;
    case "delete_project":
      projects.delete(args.path as string);
      return;
    case "close_project":
      projects.set(args.path as string, structuredClone(args.project as Project));
      return;
    case "import_clip": {
      const project = args.project as Project;
      const base = demoProject().clips[0] as RecordingClip;
      let n = 1;
      while (project.clips.some((c) => c.id === `clip${n}`)) n++;
      const clip: RecordingClip = { ...base, id: `clip${n}`, dir: `clips/clip${n}`, transition: { type: "crossfade", durationMs: 500 } };
      clip.source.video.durationMs = 8000;
      return { clip, cuts: [], zooms: [] } satisfies Imported;
    }
    case "export_presets":
      return PRESETS;
    case "music_tracks":
      return MUSIC;
    case "speech_models":
      return {
        available: true,
        models: [
          { id: "base.en", label: "Base (English)", hint: "Good accuracy, fast. The default.", megabytes: 148, englishOnly: true, downloaded: true },
          { id: "tiny.en", label: "Tiny (English)", hint: "Fastest, makes more mistakes.", megabytes: 78, englishOnly: true, downloaded: false },
        ],
      } satisfies Speech;
    case "transcribe_project":
      await job("transcribe", "listen", 16);
      return { language: "en", words: demoTranscript() };
    case "audio_preview":
      return null;
    case "suggest_cuts": {
      const project = args.project as Project;
      const total = totalDuration(project);
      const cuts = normalizeCuts(project.cuts, total);
      const words = project.transcript?.words ?? [];
      const out: Suggestion[] = [];
      if (args.fillers) {
        words.forEach((w, i) => {
          if (!/^(um|uh|er|hmm)[.,]?$/i.test(w.text) || cutAt(cuts, (w.t0 + w.t1) / 2) >= 0) return;
          const before = i ? Math.min(words[i - 1].t1, w.t0) : w.t0;
          const after = words[i + 1] ? Math.max(words[i + 1].t0, w.t1) : w.t1;
          out.push({ kind: "filler", startMs: w.t0 - Math.min(120, (w.t0 - before) / 2), endMs: w.t1 + Math.min(120, (after - w.t1) / 2), label: w.text });
        });
      }
      if (args.silences) {
        await job("silence", "listen", 6);
        const min = Number(args.minSilenceMs) || 1000;
        const edges = [0, ...words.flatMap((w) => [w.t0, w.t1]), total];
        for (let i = 0; i < edges.length; i += 2) {
          const [a, b] = [edges[i], edges[i + 1]];
          if (b - a >= min && cutAt(cuts, (a + b) / 2) < 0) out.push({ kind: "silence", startMs: a + (a === 0 ? 0 : 200), endMs: b - (b === total ? 0 : 200), label: `${((b - a) / 1000).toFixed(1)} s of silence` });
        }
      }
      return out.sort((a, b) => a.startMs - b.startMs);
    }
    case "export_captions":
      return captionLines(args.project as Project).length;
    case "list_styles":
      return structuredClone(styles);
    case "save_styles":
      styles = structuredClone(args.styles as unknown[]);
      return;
    case "get_settings":
      return structuredClone(settings);
    case "save_settings":
      settings = structuredClone(args.settings as Record<string, unknown>);
      return;
    case "share_video":
      if (!settings.shareServer || !settings.shareToken) throw new Error("set the share server address and token first");
      await job("share", "upload");
      return { id: "demo123abc", url: `${settings.shareServer}/v/demo123abc`, embedUrl: `${settings.shareServer}/embed/demo123abc`, embedHtml: `<iframe src="${settings.shareServer}/embed/demo123abc" width="800" height="450" frameborder="0" allowfullscreen></iframe>`, mediaUrl: `${settings.shareServer}/media/demo123abc.mp4` };
    case "cancel_job":
      cancelled.add(args.job as string);
      return;
    case "auto_zoom":
      return autoZoom(args.project as Project);
    case "compute_motion":
      return computeMotion(args.project as Project);
    case "export_video": {
      const total = 240;
      let frame = 0;
      window.clearInterval(exportTimer);
      return new Promise<void>((resolve, reject) => {
        exportTimer = window.setInterval(() => {
          frame += 6;
          emit("export://progress", { frame: Math.min(frame, total), total, stage: frame >= total ? "finishing" : "rendering" });
          if (frame >= total + 12) {
            window.clearInterval(exportTimer);
            resolve();
          }
        }, 50);
        cancelExport = () => {
          window.clearInterval(exportTimer);
          reject(new Error("Export cancelled"));
        };
      });
    }
    case "cancel_export":
      cancelExport?.();
      return;
    default:
      throw new Error(`Unknown command: ${cmd}`);
  }
}

let cancelExport: (() => void) | undefined;

/** A static fake desktop used as the "video" in demo mode. */
export function mockSourceCanvas(): HTMLCanvasElement {
  const c = document.createElement("canvas");
  c.width = W;
  c.height = H;
  const g = c.getContext("2d")!;
  g.fillStyle = "#f4f5f8";
  g.fillRect(0, 0, W, H);
  g.fillStyle = "#1f2430";
  g.fillRect(0, 0, W, 56);
  g.fillStyle = "#ffffff";
  g.fillRect(0, 56, 300, H - 56);
  g.fillStyle = "#e7e9ef";
  for (let i = 0; i < 9; i++) g.fillRect(28, 96 + i * 52, 180 + ((i * 37) % 60), 16);
  const cards = [
    [340, 100, 480, 300, "#6d5dfc"],
    [860, 100, 480, 300, "#22b8a0"],
    [1380, 100, 480, 300, "#f2994a"],
    [340, 440, 740, 580, "#ffffff"],
    [1120, 440, 740, 580, "#ffffff"],
  ] as const;
  for (const [x, y, w, h, col] of cards) {
    g.fillStyle = col;
    g.fillRect(x, y, w, h);
    g.fillStyle = col === "#ffffff" ? "#dfe2ea" : "rgba(255,255,255,0.75)";
    for (let i = 0; i < Math.floor(h / 60) - 1; i++) g.fillRect(x + 28, y + 36 + i * 52, w * (0.35 + ((i * 29) % 50) / 100), 14);
  }
  g.fillStyle = "#6d5dfc";
  for (const cl of CLICKS) g.fillRect(cl.x - 46, cl.y - 16, 92, 32);
  g.fillStyle = "#8a90a2";
  g.font = "600 20px system-ui, sans-serif";
  g.fillText("Demo recording", 24, 36);
  return c;
}
