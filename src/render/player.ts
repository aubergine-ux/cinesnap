// Drives playback in OUTPUT time and keeps the media elements in step with it.

import { api, isTauri, mediaUrl } from "../lib/api";
import { mockSourceCanvas } from "../lib/mock";
import { useStore } from "../lib/store";
import type { MotionTracks, Project } from "../lib/types";
import { Renderer } from "./renderer";

const DRIFT_S = 0.08;

type FrameCallbackVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: () => void) => number;
};

export class Player {
  private renderer = new Renderer();
  private ctx: CanvasRenderingContext2D;
  private video: FrameCallbackVideo | null = null;
  private mic: HTMLAudioElement | null = null;
  private webcam: HTMLVideoElement | null = null;
  private mockSource: HTMLCanvasElement | null = null;
  private backdrop: HTMLImageElement | null = null;
  private backdropPath = "";
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
    project: Project,
  ) {
    this.ctx = canvas.getContext("2d", { alpha: false })!;
    if (isTauri) {
      const v = document.createElement("video") as FrameCallbackVideo;
      v.muted = true;
      v.playsInline = true;
      v.preload = "auto";
      v.src = mediaUrl(path, project.source.video.file)!;
      this.watch(v);
      this.video = v;
      if (project.source.mic) {
        const a = document.createElement("audio");
        a.preload = "auto";
        a.src = mediaUrl(path, project.source.mic.file)!;
        this.mic = a;
      }
      if (project.source.webcam) {
        const w = document.createElement("video");
        w.muted = true;
        w.playsInline = true;
        w.preload = "auto";
        w.src = mediaUrl(path, project.source.webcam.file)!;
        this.watch(w);
        this.webcam = w;
      }
    } else {
      this.mockSource = mockSourceCanvas();
    }
    this.raf = requestAnimationFrame(this.tick);
  }

  private watch(v: FrameCallbackVideo) {
    const mark = () => (this.dirty = true);
    for (const ev of ["seeked", "loadeddata", "canplay", "timeupdate"]) v.addEventListener(ev, mark);
    // Surface decode problems (a missing system codec, usually) in the
    // backend log, where they can be found when the preview stays black.
    v.addEventListener("loadedmetadata", () => api.clientLog(`video ready ${v.videoWidth}x${v.videoHeight} ${v.duration.toFixed(2)}s`));
    v.addEventListener("error", () => api.clientLog(`video error ${v.error?.code ?? "?"}: ${v.error?.message ?? ""}`));
    if (v.requestVideoFrameCallback) {
      const loop = () => {
        if (this.disposed) return;
        this.dirty = true;
        v.requestVideoFrameCallback!(loop);
      };
      v.requestVideoFrameCallback(loop);
    }
  }

  invalidate() {
    this.dirty = true;
  }

  destroy() {
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    for (const el of [this.video, this.mic, this.webcam]) {
      if (!el) continue;
      el.pause();
      el.removeAttribute("src");
      el.load();
    }
  }

  private loadBackdrop(project: Project) {
    const bg = project.frame.background;
    const path = bg.type === "image" || bg.type === "wallpaper" ? bg.path : "";
    if (path === this.backdropPath) return;
    this.backdropPath = path;
    this.backdrop = null;
    const url = path ? mediaUrl(path) : null;
    if (!url) return;
    const img = new Image();
    img.onload = () => {
      if (this.backdropPath === path) {
        this.backdrop = img;
        this.dirty = true;
      }
    };
    img.src = url;
  }

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
      const src = motion.srcMs[Math.min(frame, motion.frames - 1)];
      const playing = this.wasPlaying;
      this.sync(this.video, src / 1000, playing, 0.4 / project.source.video.fps);
      if (project.source.mic) this.sync(this.mic, (src - project.source.mic.offsetMs) / 1000, playing, 0.05);
      if (project.source.webcam) this.sync(this.webcam, (src - project.source.webcam.offsetMs) / 1000, playing, 0.03);
    }

    if (project !== this.lastProject) this.loadBackdrop(project);
    if (project !== this.lastProject || motion !== this.lastMotion || frame !== this.lastFrame) this.dirty = true;
    this.lastProject = project;
    this.lastMotion = motion;
    this.lastFrame = frame;

    if (!this.dirty) return;
    this.dirty = false;
    const mode = motion?.cursorMode ?? st.capabilities?.cursor ?? "metadata";
    this.renderer.draw(this.ctx, this.canvas.width, this.canvas.height, project, motion, frame, {
      source: this.video ?? this.mockSource,
      backdrop: this.backdrop,
      webcam: this.webcam,
      drawCursor: mode === "metadata",
    });
  };
}
