// Typed wrappers around the backend commands. Falls back to an in-browser mock
// when the page is not running inside the desktop shell.

import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { mockInvoke, mockListen } from "./mock";
import type {
  AppSettings,
  Capabilities,
  ExportPreset,
  ExportProgress,
  Imported,
  JobProgress,
  MotionTracks,
  MusicTrack,
  Project,
  RecentProject,
  RecordingOptions,
  Shared,
  Sources,
  Speech,
  StylePreset,
  Suggestion,
  Transcript,
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

let mediaBase: string | null = null;

export const api = {
  getCapabilities: () =>
    call<Capabilities>("get_capabilities").then((caps) => {
      mediaBase = caps.mediaBase ?? null;
      return caps;
    }),
  downloadFfmpeg: () => call<void>("download_ffmpeg"),
  listSources: () => call<Sources>("list_sources"),
  startRecording: (options: RecordingOptions) => call<void>("start_recording", { options }),
  stopRecording: () => call<string>("stop_recording"),
  cancelRecording: () => call<void>("cancel_recording"),
  initialProject: () => call<string | null>("initial_project"),
  listRecent: () => call<RecentProject[]>("list_recent"),
  openProject: (path: string) => call<Project>("open_project", { path }),
  saveProject: (path: string, project: Project) => call<void>("save_project", { path, project }),
  deleteProject: (path: string) => call<void>("delete_project", { path }),
  autoZoom: (path: string, project: Project) => call<ZoomSegment[]>("auto_zoom", { path, project }),
  computeMotion: (path: string, project: Project) => call<MotionTracks>("compute_motion", { path, project }),
  exportVideo: (path: string, project: Project, outPath: string) => call<void>("export_video", { path, project, outPath }),
  cancelExport: () => call<void>("cancel_export"),
  /** Save for the last time and drop the files of clips that were removed. */
  closeProject: (path: string, project: Project) => call<void>("close_project", { path, project }),
  importClip: (path: string, project: Project, from: string) => call<Imported>("import_clip", { path, project, from }),
  exportPresets: () => call<ExportPreset[]>("export_presets"),
  musicTracks: () => call<MusicTrack[]>("music_tracks"),
  speechModels: () => call<Speech>("speech_models"),
  transcribe: (path: string, project: Project, model: string, language: string | null) => call<Transcript>("transcribe_project", { path, project, model, language }),
  /** Path of the soundtrack for the preview, or null when the project is silent. */
  audioPreview: (path: string, project: Project) => call<string | null>("audio_preview", { path, project }),
  suggestCuts: (path: string, project: Project, fillers: boolean, silences: boolean, minSilenceMs: number) => call<Suggestion[]>("suggest_cuts", { path, project, fillers, silences, minSilenceMs }),
  exportCaptions: (project: Project, outPath: string) => call<number>("export_captions", { project, outPath }),
  listStyles: () => call<StylePreset[]>("list_styles"),
  saveStyles: (styles: StylePreset[]) => call<void>("save_styles", { styles }),
  getSettings: () => call<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) => call<void>("save_settings", { settings }),
  shareVideo: (file: string, title: string) => call<Shared>("share_video", { file, title }),
  cancelJob: (job: string) => call<void>("cancel_job", { job }),
  onJobProgress: (cb: (p: JobProgress) => void) => on("job://progress", cb),
  /** Write a diagnostic line to the backend log. */
  clientLog: (message: string) => void (isTauri ? invoke("client_log", { message }).catch(() => {}) : undefined),
  onFfmpegProgress: (cb: (p: { fraction: number }) => void) => on("ffmpeg://progress", cb),
  onRecordingTick: (cb: (p: { elapsedMs: number }) => void) => on("recording://tick", cb),
  onExportProgress: (cb: (p: ExportProgress) => void) => on("export://progress", cb),
};

/** URL the webview can load for a file inside a project bundle (or any absolute path). */
export function mediaUrl(path: string, file?: string): string | null {
  if (!isTauri) return null;
  const sep = path.includes("\\") && !path.includes("/") ? "\\" : "/";
  const full = file ? `${path}${sep}${file}` : path;
  // WebKitGTK cannot play video from the asset protocol; there the backend
  // serves media over loopback HTTP instead.
  return mediaBase ? `${mediaBase}/${encodeURIComponent(full)}` : convertFileSrc(full);
}

export async function pickSavePath(defaultName: string, ext: string): Promise<string | null> {
  if (!isTauri) return `/tmp/${defaultName}.${ext}`;
  return save({ defaultPath: `${defaultName}.${ext}`, filters: [{ name: ext.toUpperCase(), extensions: [ext] }] });
}

/** Pick a file to save to; `ext` may list several extensions, first is the default. */
export async function pickSaveAs(defaultName: string, name: string, extensions: string[]): Promise<string | null> {
  if (!isTauri) return `/tmp/${defaultName}.${extensions[0]}`;
  return save({ defaultPath: `${defaultName}.${extensions[0]}`, filters: [{ name, extensions }] });
}

export async function pickAudio(): Promise<string | null> {
  if (!isTauri) return null;
  const res = await open({ multiple: false, filters: [{ name: "Audio", extensions: ["mp3", "wav", "m4a", "aac", "ogg", "flac", "opus"] }] });
  return typeof res === "string" ? res : null;
}

/** Pick a .cinesnap recording folder. */
export async function pickRecording(): Promise<string | null> {
  if (!isTauri) return "mock://other.cinesnap";
  const res = await open({ directory: true, multiple: false });
  return typeof res === "string" ? res : null;
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
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
