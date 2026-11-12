// In-browser stand-in for the Rust backend, used when the app runs outside Tauri
// (plain `npm run dev`). It serves a demo project and synthetic motion tracks.

import { clampCentre, outputFps } from "./layout";
import { cutAt, keptRanges, normalizeCuts, outToSrc, srcToOut } from "./time";
import type {
  Capabilities,
  MotionTracks,
  Project,
  RecentProject,
  RecordingOptions,
  Sources,
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

function defaultProject(): Project {
  return {
    format: "cinesnap-project",
    version: 1,
    createdAt: new Date().toISOString(),
    app: "0.1.0",
    source: {
      video: { file: "video.mp4", width: W, height: H, fps: 60, durationMs: DURATION },
      events: { file: "events.jsonl" },
      platform: "mock",
      target: "screen",
    },
    crop: { x: 0, y: 0, width: W, height: H },
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
  };
}

const projects = new Map<string, Project>();

function getProject(path: string): Project {
  if (!projects.has(path)) {
    const p = defaultProject();
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

function computeMotion(project: Project): MotionTracks {
  const duration = project.source.video.durationMs;
  const fps = outputFps(project.export);
  const cuts = normalizeCuts(project.cuts, duration);
  const outDuration = keptRanges(cuts, duration).reduce((s, r) => s + r.endMs - r.startMs, 0);
  const frames = Math.max(1, Math.floor((outDuration / 1000) * fps));
  const srcMs = new Array<number>(frames);
  const cam = new Array<number>(frames * 3);
  const cursor = new Array<number>(frames * 4);
  const crop = project.crop;
  const cx0 = crop.x + crop.width / 2;
  const cy0 = crop.y + crop.height / 2;
  for (let i = 0; i < frames; i++) {
    const src = outToSrc((i * 1000) / fps, cuts, duration);
    srcMs[i] = src;
    const pos = cursorAt(src);
    let k = 0;
    let scale = 1;
    let tx = cx0;
    let ty = cy0;
    for (const z of project.zooms) {
      if (src < z.startMs || src > z.endMs) continue;
      k = Math.min(ease(z.easing, (src - z.startMs) / Math.max(1, z.easeInMs)), ease(z.easing, (z.endMs - src) / Math.max(1, z.easeOutMs)));
      scale = 1 + (z.scale - 1) * k;
      tx = z.mode === "follow" ? pos.x : z.x;
      ty = z.mode === "follow" ? pos.y : z.y;
    }
    const c = clampCentre(crop, cx0 + (tx - cx0) * k, cy0 + (ty - cy0) * k, scale);
    cam[i * 3] = c.cx;
    cam[i * 3 + 1] = c.cy;
    cam[i * 3 + 2] = scale;
    let press = 0;
    for (const cl of CLICKS) if (src >= cl.t && src < cl.t + 160) press = 1 - (src - cl.t) / 160;
    cursor[i * 4] = pos.x;
    cursor[i * 4 + 1] = pos.y;
    cursor[i * 4 + 2] = 1;
    cursor[i * 4 + 3] = press;
  }
  return {
    fps,
    frames,
    durationMs: outDuration,
    srcMs,
    cam,
    cursor,
    ripples: CLICKS.filter((c) => cutAt(cuts, c.t) < 0).map((c) => ({ outMs: srcToOut(c.t, cuts, duration), x: c.x, y: c.y })),
    events: { clicks: CLICKS.map((c) => c.t), keys: KEYS },
    cursorMode: "metadata",
  };
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
