// Drives playback in OUTPUT time and keeps the media elements in step with it.

import { api, isTauri, mediaUrl } from "../lib/api";
import { mockSourceCanvas } from "../lib/mock";
import { useStore } from "../lib/store";
import type { MotionTracks, Project, RecordingClip } from "../lib/types";
import { loadFonts } from "./text";
import { Renderer, type ClipMedia } from "./renderer";

const DRIFT_S = 0.08;

type FrameCallbackVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: () => void) => number;
};

interface Media {
  video: FrameCallbackVideo | null;
  webcam: HTMLVideoElement | null;
}

/** Folder of a clip's files. */
function clipPath(path: string, clip: RecordingClip): string {
  if (!clip.dir) return path;
  const sep = path.includes("\\") && !path.includes("/") ? "\\" : "/";
  return `${path}${sep}${clip.dir.split("/").join(sep)}`;
}

/** First and last timeline time each clip shows, from the motion tracks. */
function clipRanges(motion: MotionTracks): Map<number, { first: number; last: number }> {
  const out = new Map<number, { first: number; last: number }>();
  for (let i = 0; i < motion.frames; i++) {
    const c = motion.clip[i];
    const r = out.get(c);
    if (r) r.last = motion.srcMs[i];
    else out.set(c, { first: motion.srcMs[i], last: motion.srcMs[i] });
  }
  return out;
}

export class Player {
  private renderer = new Renderer();
  private ctx: CanvasRenderingContext2D;
  /** Media elements per clip id. */
  private media = new Map<string, Media>();
  private audio: HTMLAudioElement | null = null;
  private audioPath: string | null = null;
  private mockSource: HTMLCanvasElement | null = null;
  private backdrops = new Map<string, HTMLImageElement | null>();
  private ranges = new Map<number, { first: number; last: number }>();
  private raf = 0;
  private dirty = true;
  private wasPlaying = false;
  private clockMs = 0;
  private lastNow = 0;
  private lastFrame = -1;
  private lastProject: Project | null = null;
  private lastMotion: MotionTracks | null = null;
  private disposed = false;

  constructor(
    private canvas: HTMLCanvasElement,
    private path: string,
  ) {
    this.ctx = canvas.getContext("2d", { alpha: false })!;
    if (isTauri) {
      this.audio = document.createElement("audio");
      this.audio.preload = "auto";
    } else {
      this.mockSource = mockSourceCanvas();
    }
    void loadFonts().then(() => (this.dirty = true));
    this.raf = requestAnimationFrame(this.tick);
  }

  private watch(v: FrameCallbackVideo, label: string) {
    const mark = () => (this.dirty = true);
    for (const ev of ["seeked", "loadeddata", "canplay", "timeupdate"]) v.addEventListener(ev, mark);
    // Surface decode problems (a missing system codec, usually) in the
    // backend log, where they can be found when the preview stays black.
    v.addEventListener("loadedmetadata", () => api.clientLog(`${label} ready ${v.videoWidth}x${v.videoHeight} ${v.duration.toFixed(2)}s`));
    v.addEventListener("error", () => api.clientLog(`${label} error ${v.error?.code ?? "?"}: ${v.error?.message ?? ""}`));
    if (v.requestVideoFrameCallback) {
      const loop = () => {
        if (this.disposed) return;
        this.dirty = true;
        v.requestVideoFrameCallback!(loop);
      };
      v.requestVideoFrameCallback(loop);
    }
  }

  private makeVideo(url: string, label: string): FrameCallbackVideo {
    const v = document.createElement("video") as FrameCallbackVideo;
    v.muted = true;
    v.playsInline = true;
    v.preload = "auto";
    v.src = url;
    this.watch(v, label);
    return v;
  }

  private release(el: HTMLMediaElement | null) {
    if (!el) return;
    el.pause();
    el.removeAttribute("src");
    el.load();
  }

  /** Create media for new clips, drop it for removed ones. */
  private reconcile(project: Project) {
    const wanted = new Set<string>();
    for (const clip of project.clips) {
      if (clip.kind !== "recording") continue;
      wanted.add(clip.id);
      if (this.media.has(clip.id) || !isTauri) continue;
      const dir = clipPath(this.path, clip);
      this.media.set(clip.id, {
        video: this.makeVideo(mediaUrl(dir, clip.source.video.file)!, `video ${clip.id}`),
        webcam: clip.source.webcam ? this.makeVideo(mediaUrl(dir, clip.source.webcam.file)!, `webcam ${clip.id}`) : null,
      });
    }
    for (const [id, m] of this.media) {
      if (wanted.has(id)) continue;
      this.release(m.video);
      this.release(m.webcam);
      this.media.delete(id);
    }
  }

  invalidate() {
    this.dirty = true;
  }

  destroy() {
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    for (const m of this.media.values()) {
      this.release(m.video);
      this.release(m.webcam);
    }
    this.release(this.audio);
  }

  private backdrop = (path: string): HTMLImageElement | null => {
    if (this.backdrops.has(path)) return this.backdrops.get(path) ?? null;
    this.backdrops.set(path, null);
    const url = mediaUrl(path);
    if (url) {
      const img = new Image();
      img.onload = () => {
        this.backdrops.set(path, img);
        this.dirty = true;
      };
      img.src = url;
    }
    return null;
  };

  private sync(el: HTMLMediaElement | null, targetS: number, playing: boolean, pausedTolerance: number) {
    if (!el || el.readyState < 1) return;
    const inRange = targetS >= 0 && (!Number.isFinite(el.duration) || targetS < el.duration);
    if (playing && inRange) {
      if (el.paused) {
        el.currentTime = targetS;
        el.play().catch(() => undefined);
      } else if (Math.abs(el.currentTime - targetS) > DRIFT_S) {
        el.currentTime = targetS;
      }
    } else {
      if (!el.paused) el.pause();
      if (inRange && !el.seeking && Math.abs(el.currentTime - targetS) > pausedTolerance) el.currentTime = targetS;
    }
  }

  private syncAudio(outS: number, playing: boolean, path: string | null, stale: boolean) {
    const a = this.audio;
    if (!a) return;
    if (path !== this.audioPath) {
      this.audioPath = path;
      if (path) a.src = mediaUrl(path)!;
      else this.release(a);
    }
    // A soundtrack that no longer matches the edits would be out of sync.
    this.sync(path ? a : null, outS, playing && !stale, 0.05);
    if (stale && !a.paused) a.pause();
  }

  private tick = (now: number) => {
    if (this.disposed) return;
    this.raf = requestAnimationFrame(this.tick);
    const st = useStore.getState();
    const { project, motion } = st;
    if (!project || st.projectPath !== this.path) return;

    // Keep the backing store at device resolution.
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const w = Math.max(2, Math.round(this.canvas.clientWidth * dpr));
    const h = Math.max(2, Math.round(this.canvas.clientHeight * dpr));
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
      this.dirty = true;
    }

    if (project.clips !== this.lastProject?.clips) this.reconcile(project);
    if (motion !== this.lastMotion) this.ranges = motion ? clipRanges(motion) : new Map();

    let frame = st.frame;
    if (st.playing && motion && motion.frames > 0) {
      // Restart the clock when playback starts or the playhead was moved externally.
      if (!this.wasPlaying || frame !== this.lastFrame) this.clockMs = (frame * 1000) / motion.fps;
      else this.clockMs += now - this.lastNow;
      frame = Math.floor((this.clockMs / 1000) * motion.fps);
      if (frame >= motion.frames - 1) {
        frame = motion.frames - 1;
        st.setPlaying(false);
      }
      if (frame !== st.frame) st.setFrame(frame);
      this.dirty = true;
    }
    this.wasPlaying = useStore.getState().playing;
    this.lastNow = now;

    if (motion && motion.frames > 0) {
      const i = Math.min(frame, motion.frames - 1);
      const current = motion.clip[i];
      const playing = this.wasPlaying;
      project.clips.forEach((clip, index) => {
        if (clip.kind !== "recording") return;
        const m = this.media.get(clip.id);
        if (!m) return;
        const start = motion.clipStarts[index] ?? 0;
        const range = this.ranges.get(index);
        // The clip on screen follows the playhead. The others wait where a
        // transition will need them: later clips on their first frame,
        // earlier ones on their last.
        const t = index === current ? motion.srcMs[i] : index > current ? (range?.first ?? start) : (range?.last ?? start);
        const local = (t - start) / 1000;
        const live = index === current && playing;
        this.sync(m.video, local, live, 0.4 / clip.source.video.fps);
        if (clip.source.webcam) this.sync(m.webcam, local - clip.source.webcam.offsetMs / 1000, live, 0.03);
      });
      this.syncAudio((i * 1000) / motion.fps / 1000, playing, st.audioPath, st.audioStale);
    }

    if (project !== this.lastProject || motion !== this.lastMotion || frame !== this.lastFrame) this.dirty = true;
    this.lastProject = project;
    this.lastMotion = motion;
    this.lastFrame = frame;

    if (!this.dirty) return;
    this.dirty = false;
    const clips: (ClipMedia | undefined)[] = project.clips.map((clip) => {
      if (clip.kind !== "recording") return undefined;
      const m = this.media.get(clip.id);
      return { source: m?.video ?? this.mockSource, webcam: m?.webcam ?? null };
    });
    this.renderer.draw(this.ctx, this.canvas.width, this.canvas.height, project, motion, frame, { clips, backdrop: this.backdrop });
  };
}
