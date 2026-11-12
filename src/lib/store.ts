import { create } from "zustand";
import { api, errorMessage, isTauri } from "./api";
import { addCut, newZoomRange, normalizeCuts, srcToOut } from "./time";
import type { Capabilities, MotionTracks, Project, RecentProject, Sources, ZoomSegment } from "./types";

export type Selection = { kind: "zoom"; id: string } | { kind: "cut"; startMs: number } | null;

interface UpdateOptions {
  /** Edits with the same key made in quick succession share one undo step. */
  key?: string;
  /** Skip the undo stack entirely. */
  silent?: boolean;
}

interface State {
  screen: "home" | "recording" | "editor";
  capabilities: Capabilities | null;
  sources: Sources | null;
  recent: RecentProject[];
  error: string | null;

  projectPath: string | null;
  project: Project | null;
  motion: MotionTracks | null;
  past: Project[];
  future: Project[];
  selection: Selection;
  playing: boolean;
  /** Playhead as an OUTPUT frame index. */
  frame: number;
  cutMode: boolean;
  /** Pending cut-in point (source ms) set with the S key. */
  cutMark: number | null;
  panelOpen: boolean;

  init: () => Promise<void>;
  refreshHome: () => Promise<void>;
  setError: (e: string | null) => void;
  setScreen: (s: State["screen"]) => void;
  openProject: (path: string) => Promise<void>;
  closeProject: () => void;
  update: (fn: (p: Project) => void, opts?: UpdateOptions) => void;
  undo: () => void;
  redo: () => void;
  select: (s: Selection) => void;
  deleteSelection: () => void;
  setPlaying: (p: boolean) => void;
  setFrame: (f: number) => void;
  seekSource: (ms: number) => void;
  sourceTime: () => number;
  setCutMode: (on: boolean) => void;
  markOrCut: () => void;
  addZoomAt: (ms: number) => void;
  editZoom: (id: string, patch: Partial<ZoomSegment>, key?: string) => void;
  regenerateZooms: () => Promise<void>;
  togglePanel: () => void;
}

const HISTORY_LIMIT = 200;
const COALESCE_MS = 700;
let lastKey: string | undefined;
let lastAt = 0;
let motionTimer: ReturnType<typeof setTimeout> | undefined;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let motionKey = "";
let motionSeq = 0;

function motionInputs(p: Project): string {
  return JSON.stringify([p.zooms, p.cuts, p.cursor.smoothing, p.cursor.hideWhenIdleMs, p.export.fps, p.export.format, p.export.preset, p.crop]);
}

/** First output frame whose source time is at or after `ms`. */
export function frameForSource(motion: MotionTracks, ms: number): number {
  let lo = 0;
  let hi = motion.frames - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (motion.srcMs[mid] < ms) lo = mid + 1;
    else hi = mid;
  }
  return Math.max(0, lo);
}

export const useStore = create<State>((set, get) => {
  const scheduleMotion = (immediate = false) => {
    const { project, projectPath } = get();
    if (!project || !projectPath) return;
    const key = motionInputs(project);
    if (key === motionKey && get().motion) return;
    clearTimeout(motionTimer);
    const run = async () => {
      const cur = get();
      if (!cur.project || cur.projectPath !== projectPath) return;
      const seq = ++motionSeq;
      const k = motionInputs(cur.project);
      try {
        const motion = await api.computeMotion(projectPath, cur.project);
        if (seq !== motionSeq || get().projectPath !== projectPath) return;
        motionKey = k;
        const prev = get().motion;
        const src = prev && prev.frames ? prev.srcMs[Math.min(get().frame, prev.frames - 1)] : 0;
        set({ motion, frame: Math.min(frameForSource(motion, src), Math.max(0, motion.frames - 1)) });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    };
    if (immediate) void run();
    else motionTimer = setTimeout(run, 30);
  };

  const scheduleSave = () => {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      const { project, projectPath } = get();
      if (project && projectPath) api.saveProject(projectPath, project).catch((e) => set({ error: errorMessage(e) }));
    }, 500);
  };

  const changed = () => {
    scheduleMotion();
    scheduleSave();
  };

  return {
    screen: "home",
    capabilities: null,
    sources: null,
    recent: [],
    error: null,
    projectPath: null,
    project: null,
    motion: null,
    past: [],
    future: [],
    selection: null,
    playing: false,
    frame: 0,
    cutMode: false,
    cutMark: null,
    panelOpen: true,

    init: async () => {
      await get().refreshHome();
      // Browser demo: /#demo opens the sample project straight away.
      if (!isTauri && window.location.hash === "#demo" && get().recent[0]) await get().openProject(get().recent[0].path);
    },

    refreshHome: async () => {
      try {
        const [capabilities, sources, recent] = await Promise.all([api.getCapabilities(), api.listSources(), api.listRecent()]);
        set({ capabilities, sources, recent });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    setError: (error) => set({ error }),
    setScreen: (screen) => set({ screen }),

    openProject: async (path) => {
      try {
        const project = await api.openProject(path);
        project.cuts = normalizeCuts(project.cuts ?? [], project.source.video.durationMs);
        motionKey = "";
        lastKey = undefined;
        set({
          screen: "editor",
          projectPath: path,
          project,
          motion: null,
          past: [],
          future: [],
          selection: null,
          playing: false,
          frame: 0,
          cutMode: false,
          cutMark: null,
        });
        scheduleMotion(true);
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    closeProject: () => {
      const { project, projectPath } = get();
      clearTimeout(saveTimer);
      clearTimeout(motionTimer);
      if (project && projectPath) api.saveProject(projectPath, project).catch(() => undefined);
      set({ screen: "home", project: null, projectPath: null, motion: null, playing: false, past: [], future: [], selection: null });
      void get().refreshHome();
    },

    update: (fn, opts = {}) => {
      const { project, past } = get();
      if (!project) return;
      const next = structuredClone(project);
      fn(next);
      const now = Date.now();
      const coalesce = opts.silent || (opts.key !== undefined && opts.key === lastKey && now - lastAt < COALESCE_MS);
      lastKey = opts.silent ? lastKey : opts.key;
      lastAt = now;
      set({
        project: next,
        past: coalesce ? past : [...past.slice(-HISTORY_LIMIT + 1), project],
        future: opts.silent ? get().future : [],
      });
      changed();
    },

    undo: () => {
      const { past, future, project } = get();
      if (!project || !past.length) return;
      lastKey = undefined;
      set({ project: past[past.length - 1], past: past.slice(0, -1), future: [project, ...future] });
      changed();
    },

    redo: () => {
      const { past, future, project } = get();
      if (!project || !future.length) return;
      lastKey = undefined;
      set({ project: future[0], past: [...past, project], future: future.slice(1) });
      changed();
    },

    select: (selection) => set({ selection }),

    deleteSelection: () => {
      const sel = get().selection;
      if (!sel) return;
      if (sel.kind === "zoom") get().update((p) => void (p.zooms = p.zooms.filter((z) => z.id !== sel.id)));
      else get().update((p) => void (p.cuts = p.cuts.filter((c) => Math.abs(c.startMs - sel.startMs) > 0.5)));
      set({ selection: null });
    },

    setPlaying: (playing) => {
      const { motion, frame } = get();
      if (playing && motion && frame >= motion.frames - 1) set({ frame: 0 });
      set({ playing });
    },

    setFrame: (frame) => {
      const m = get().motion;
      const f = Math.max(0, Math.min(m ? m.frames - 1 : 0, Math.round(frame)));
      if (f !== get().frame) set({ frame: f });
    },

    seekSource: (ms) => {
      const m = get().motion;
      if (m) get().setFrame(frameForSource(m, ms));
    },

    sourceTime: () => {
      const { motion, frame } = get();
      return motion && motion.frames ? motion.srcMs[Math.min(frame, motion.frames - 1)] : 0;
    },

    setCutMode: (cutMode) => set({ cutMode }),

    markOrCut: () => {
      const { project, cutMark } = get();
      if (!project) return;
      const now = get().sourceTime();
      if (cutMark === null) {
        set({ cutMark: now });
        return;
      }
      const a = Math.min(cutMark, now);
      const b = Math.max(cutMark, now);
      set({ cutMark: null });
      if (b - a < 50) return;
      get().update((p) => void (p.cuts = addCut(p.cuts, { startMs: a, endMs: b }, p.source.video.durationMs)));
      get().seekSource(b);
    },

    addZoomAt: (ms) => {
      const { project } = get();
      if (!project) return;
      const range = newZoomRange(project.zooms, ms, project.source.video.durationMs);
      if (!range) return;
      const id = `m${Date.now().toString(36)}${Math.floor(Math.random() * 1e4).toString(36)}`;
      const seg: ZoomSegment = {
        id,
        ...range,
        scale: project.autoZoom.scale,
        easeInMs: 600,
        easeOutMs: 700,
        easing: "smooth",
        mode: "follow",
        x: project.crop.x + project.crop.width / 2,
        y: project.crop.y + project.crop.height / 2,
        manual: true,
      };
      get().update((p) => {
        p.zooms.push(seg);
        p.zooms.sort((a, b) => a.startMs - b.startMs);
      });
      set({ selection: { kind: "zoom", id } });
    },

    editZoom: (id, patch, key) => {
      get().update(
        (p) => {
          const z = p.zooms.find((s) => s.id === id);
          if (z) Object.assign(z, patch, { manual: true });
        },
        { key: key ? `zoom:${id}:${key}` : undefined },
      );
    },

    regenerateZooms: async () => {
      const { project, projectPath } = get();
      if (!project || !projectPath) return;
      try {
        const zooms = await api.autoZoom(projectPath, project);
        if (get().projectPath !== projectPath) return;
        get().update((p) => void (p.zooms = zooms), { key: "auto-zoom" });
        const sel = get().selection;
        if (sel?.kind === "zoom" && !zooms.some((z) => z.id === sel.id)) set({ selection: null });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    togglePanel: () => set({ panelOpen: !get().panelOpen }),
  };
});

/** Output time (ms) of a source time for the current project. */
export function sourceToOutput(project: Project, ms: number): number {
  return srcToOut(ms, project.cuts, project.source.video.durationMs);
}
