// Typed wrappers around the backend commands. Falls back to an in-browser mock
// when the page is not running inside the desktop shell.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { mockInvoke, mockListen } from "./mock";
import type {
  Capabilities,
  ExportProgress,
  MotionTracks,
  Project,
  RecentProject,
  RecordingOptions,
  Sources,
  ZoomSegment,
} from "./types";

export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return (isTauri ? invoke<T>(cmd, args) : mockInvoke(cmd, args)) as Promise<T>;
}

/** Subscribe to a backend event. Returns an unsubscribe function. */
export function on<T>(event: string, cb: (payload: T) => void): () => void {
  if (!isTauri) return mockListen(event, (p) => cb(p as T));
  let off: (() => void) | undefined;
  let cancelled = false;
  void listen<T>(event, (e) => cb(e.payload)).then((un) => {
    if (cancelled) un();
    else off = un;
  });
  return () => {
    cancelled = true;
    off?.();
  };
}

export const api = {
  getCapabilities: () => call<Capabilities>("get_capabilities"),
  downloadFfmpeg: () => call<void>("download_ffmpeg"),
  listSources: () => call<Sources>("list_sources"),
  startRecording: (options: RecordingOptions) => call<void>("start_recording", { options }),
  stopRecording: () => call<string>("stop_recording"),
  cancelRecording: () => call<void>("cancel_recording"),
  listRecent: () => call<RecentProject[]>("list_recent"),
  openProject: (path: string) => call<Project>("open_project", { path }),
  saveProject: (path: string, project: Project) => call<void>("save_project", { path, project }),
  deleteProject: (path: string) => call<void>("delete_project", { path }),
  autoZoom: (path: string, project: Project) => call<ZoomSegment[]>("auto_zoom", { path, project }),
  computeMotion: (path: string, project: Project) => call<MotionTracks>("compute_motion", { path, project }),
  exportVideo: (path: string, project: Project, outPath: string) => call<void>("export_video", { path, project, outPath }),
  cancelExport: () => call<void>("cancel_export"),
  onFfmpegProgress: (cb: (p: { fraction: number }) => void) => on("ffmpeg://progress", cb),
  onRecordingTick: (cb: (p: { elapsedMs: number }) => void) => on("recording://tick", cb),
  onExportProgress: (cb: (p: ExportProgress) => void) => on("export://progress", cb),
};

/** URL the webview can load for a file inside a project bundle (or any absolute path). */
export function mediaUrl(path: string, file?: string): string | null {
  if (!isTauri) return null;
  const sep = path.includes("\\") && !path.includes("/") ? "\\" : "/";
  return convertFileSrc(file ? `${path}${sep}${file}` : path);
}

export async function pickSavePath(defaultName: string, ext: string): Promise<string | null> {
  if (!isTauri) return `/tmp/${defaultName}.${ext}`;
  return save({ defaultPath: `${defaultName}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
}

export async function pickImage(): Promise<string | null> {
  if (!isTauri) return null;
  const res = await open({ multiple: false, filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp"] }] });
  return typeof res === "string" ? res : null;
}

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
