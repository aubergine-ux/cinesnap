import { create } from "zustand";
import { api, errorMessage, isTauri } from "./api";
import { BUILTIN_STYLES, appendImported, applyStyle, extractStyle, insertClip, moveClip, newAnnotation, newCard, newId, removeClip, restoreWords, shiftFrom, spanAt, spans, totalDuration, webcamStateAt, wordsCut } from "./edit";
import { viewRect, type Box } from "./layout";
import { addCut, newZoomRange, normalizeCuts } from "./time";
import type { Annotation, AnnotationType, AppSettings, Capabilities, Clip, Cut, JobProgress, MotionTracks, MusicTrack, Project, RecentProject, Sources, Speech, StylePreset, WebcamKey, ZoomSegment } from "./types";

export type Selection =
  | { kind: "zoom"; id: string }
  | { kind: "cut"; startMs: number }
  | { kind: "annotation"; id: string }
  | { kind: "clip"; id: string }
  | { kind: "webcamKey"; id: string }
  | null;

export type PanelTab = "style" | "video" | "audio" | "captions" | "export";
export type SaveState = "saved" | "dirty" | "saving" | "error";

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
  panelTab: PanelTab;
  /** Whether the project on disk matches what is on screen. */
  saveState: SaveState;
  /** Soundtrack of the preview (a file path), when one has been built. */
  audioPath: string | null;
  /** The soundtrack no longer matches the edits and is being rebuilt. */
  audioStale: boolean;
  /** Progress of background jobs, by kind. */
  jobs: Record<string, JobProgress>;
  styles: StylePreset[];
  settings: AppSettings;
  musicTracks: MusicTrack[];
  speech: Speech | null;
  /** Selected words in the transcript: first and last index. */
  wordSel: [number, number] | null;

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
  setTab: (tab: PanelTab) => void;
  /** Write pending edits to disk now. */
  flushSave: () => Promise<void>;
  /** Length of the timeline before cuts. */
  duration: () => number;
  /** The part of the recording visible at the playhead, in source pixels. */
  currentView: () => Box | null;

  addAnnotation: (type: AnnotationType) => void;
  editAnnotation: (id: string, patch: Partial<Annotation>, key?: string) => void;
  addCard: (where: "start" | "end" | "here") => void;
  addRecording: (from: string) => Promise<void>;
  editClip: (id: string, patch: Partial<Clip>, key?: string) => void;
  removeClip: (id: string) => void;
  moveClip: (id: string, by: number) => void;
  addWebcamKey: () => void;
  editWebcamKey: (id: string, patch: Partial<WebcamKey>, key?: string) => void;

  selectWords: (sel: [number, number] | null) => void;
  cutWords: (i: number, j: number) => void;
  restoreWords: (i: number, j: number) => void;
  editWord: (i: number, text: string) => void;
  transcribe: (model: string, language: string | null) => Promise<void>;
  applyCuts: (cuts: Cut[]) => void;
  cancelJob: (job: string) => void;

  applyStyle: (style: StylePreset) => void;
  saveStyle: (name: string) => Promise<void>;
  deleteStyle: (name: string) => Promise<void>;
  saveSettings: (patch: Partial<AppSettings>) => Promise<void>;
}

const HISTORY_LIMIT = 200;
const COALESCE_MS = 700;
let lastKey: string | undefined;
let lastAt = 0;
let motionTimer: ReturnType<typeof setTimeout> | undefined;
let saveTimer: ReturnType<typeof setTimeout> | undefined;
let audioTimer: ReturnType<typeof setTimeout> | undefined;
let motionKey = "";
let motionSeq = 0;
let audioKey = "";
let audioSeq = 0;
let saving: Promise<void> = Promise.resolve();
/** The final save of the project that was closed last. */
let closed: Promise<void> = Promise.resolve();

function clipShape(c: Clip) {
  return c.kind === "card" ? [c.id, c.durationMs, c.transition] : [c.id, c.source.video.durationMs, c.transition, c.crop, Boolean(c.source.webcam)];
}

function motionInputs(p: Project): string {
  return JSON.stringify([
    p.zooms,
    p.cuts,
    p.cursor.smoothing,
    p.cursor.hideWhenIdleMs,
    p.cursor.show,
    p.export.fps,
    p.export.format,
    p.export.preset,
    p.frame.aspect,
    p.clips.map(clipShape),
    p.webcam,
    p.webcamKeys,
    p.captions.maxChars,
    p.transcript?.words,
  ]);
}

/** Everything the soundtrack depends on. */
function audioInputs(p: Project): string {
  return JSON.stringify([p.cuts, p.audio, p.clips.map((c) => [c.id, c.kind === "card" ? c.durationMs : c.source.video.durationMs])]);
}

function hasSound(p: Project): boolean {
  return Boolean(p.audio.music?.source) || p.clips.some((c) => c.kind === "recording" && c.source.mic);
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

  // Saves run one after another so an older save can never land on top of
  // a newer one.
  const saveNow = (): Promise<void> => {
    clearTimeout(saveTimer);
    const { project, projectPath } = get();
    if (!project || !projectPath) return saving;
    set({ saveState: "saving" });
    saving = saving
      .catch(() => undefined)
      .then(() => api.saveProject(projectPath, project))
      .then(
        () => {
          if (get().project === project) set({ saveState: "saved" });
        },
        (e) => set({ saveState: "error", error: `Could not save: ${errorMessage(e)}` }),
      );
    return saving;
  };

  const scheduleSave = () => {
    clearTimeout(saveTimer);
    set({ saveState: "dirty" });
    saveTimer = setTimeout(() => void saveNow(), 400);
  };

  const scheduleAudio = (immediate = false) => {
    const { project, projectPath } = get();
    if (!project || !projectPath) return;
    const key = audioInputs(project);
    if (key === audioKey) return;
    clearTimeout(audioTimer);
    if (!hasSound(project)) {
      audioKey = key;
      audioSeq++;
      set({ audioPath: null, audioStale: false });
      return;
    }
    set({ audioStale: true });
    const run = async () => {
      const cur = get();
      if (!cur.project || cur.projectPath !== projectPath) return;
      const seq = ++audioSeq;
      const k = audioInputs(cur.project);
      try {
        const path = await api.audioPreview(projectPath, cur.project);
        if (seq !== audioSeq || get().projectPath !== projectPath) return;
        audioKey = k;
        set({ audioPath: path, audioStale: false });
      } catch (e) {
        if (seq !== audioSeq) return;
        // A newer request cancels the one before it; that is not an error.
        if (errorMessage(e) !== "cancelled") set({ error: errorMessage(e), audioStale: false, audioPath: null });
      } finally {
        if (seq === audioSeq) set((s) => ({ jobs: omit(s.jobs, "audio") }));
      }
    };
    if (immediate) void run();
    else audioTimer = setTimeout(run, 600);
  };

  const changed = () => {
    scheduleMotion();
    scheduleSave();
    scheduleAudio();
  };

  const userStyles = () => get().styles.filter((s) => !BUILTIN_STYLES.some((b) => b.name === s.name));

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
    panelTab: "style",
    saveState: "saved",
    audioPath: null,
    audioStale: false,
    jobs: {},
    styles: BUILTIN_STYLES,
    settings: {},
    musicTracks: [],
    speech: null,
    wordSel: null,

    init: async () => {
      api.onJobProgress((p) => set((s) => ({ jobs: { ...s.jobs, [p.job]: p } })));
      if (typeof window !== "undefined") {
        // Closing the window or switching away must never lose an edit.
        window.addEventListener("beforeunload", () => void get().flushSave());
        document.addEventListener("visibilitychange", () => {
          if (document.visibilityState === "hidden") void get().flushSave();
        });
      }
      void Promise.all([api.listStyles(), api.getSettings(), api.musicTracks(), api.speechModels()]).then(
        ([styles, settings, musicTracks, speech]) => set({ styles: [...BUILTIN_STYLES, ...styles.filter((s) => !BUILTIN_STYLES.some((b) => b.name === s.name))], settings, musicTracks, speech }),
        () => undefined,
      );
      await get().refreshHome();
      // Browser demo: /#demo opens the sample project straight away.
      if (!isTauri && window.location.hash === "#demo" && get().recent[0]) await get().openProject(get().recent[0].path);
      // `cinesnap path/to/recording.cinesnap` opens straight into the editor.
      const initial = isTauri ? await api.initialProject().catch(() => null) : null;
      if (initial) await get().openProject(initial);
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
        // Never read a project while its last save is still on the way.
        await closed;
        const project = await api.openProject(path);
        project.cuts = normalizeCuts(project.cuts ?? [], totalDuration(project));
        motionKey = "";
        audioKey = "";
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
          saveState: "saved",
          audioPath: null,
          audioStale: false,
          wordSel: null,
        });
        scheduleMotion(true);
        scheduleAudio(true);
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    closeProject: () => {
      const { project, projectPath } = get();
      clearTimeout(saveTimer);
      clearTimeout(motionTimer);
      clearTimeout(audioTimer);
      audioSeq++;
      // Undo history ends here, so removed clips can be deleted for good.
      const closing = project && projectPath ? saving.catch(() => undefined).then(() => api.closeProject(projectPath, project)) : Promise.resolve();
      set({ screen: "home", project: null, projectPath: null, motion: null, playing: false, past: [], future: [], selection: null, audioPath: null, wordSel: null, saveState: "saved" });
      closed = closing.catch((e) => set({ error: `Could not save: ${errorMessage(e)}` }));
      void closed.then(() => get().refreshHome());
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
      else if (sel.kind === "cut") get().update((p) => void (p.cuts = p.cuts.filter((c) => Math.abs(c.startMs - sel.startMs) > 0.5)));
      else if (sel.kind === "annotation") get().update((p) => void (p.annotations = p.annotations.filter((a) => a.id !== sel.id)));
      else if (sel.kind === "webcamKey") get().update((p) => void (p.webcamKeys = p.webcamKeys.filter((k) => k.id !== sel.id)));
      else return get().removeClip(sel.id);
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
      get().update((p) => void (p.cuts = addCut(p.cuts, { startMs: a, endMs: b }, totalDuration(p))));
      get().seekSource(b);
    },

    addZoomAt: (ms) => {
      const { project } = get();
      if (!project) return;
      const range = newZoomRange(project.zooms, ms, totalDuration(project));
      if (!range) return;
      const id = `m${Date.now().toString(36)}${Math.floor(Math.random() * 1e4).toString(36)}`;
      // A zoom belongs to one recording; keep it inside the clip it starts in.
      const span = spanAt(project, range.startMs);
      const clip = span && project.clips[span.index];
      if (!span || !clip || clip.kind !== "recording") return;
      range.endMs = Math.min(range.endMs, span.endMs);
      if (range.endMs - range.startMs < 300) return;
      const seg: ZoomSegment = {
        id,
        ...range,
        scale: project.autoZoom.scale,
        easeInMs: 600,
        easeOutMs: 700,
        easing: "smooth",
        mode: "follow",
        x: clip.crop.x + clip.crop.width / 2,
        y: clip.crop.y + clip.crop.height / 2,
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
    setTab: (panelTab) => set({ panelTab, panelOpen: true }),
    flushSave: () => (get().saveState === "dirty" ? saveNow() : saving),
    duration: () => {
      const p = get().project;
      return p ? totalDuration(p) : 0;
    },

    currentView: () => {
      const { project, motion, frame } = get();
      if (!project) return null;
      const span = spanAt(project, get().sourceTime());
      const clip = span && project.clips[span.index];
      if (!clip || clip.kind !== "recording") return null;
      const i = motion && motion.frames ? Math.min(frame, motion.frames - 1) : -1;
      if (i < 0) return viewRect(clip.crop, clip.crop.x + clip.crop.width / 2, clip.crop.y + clip.crop.height / 2, 1);
      return viewRect(clip.crop, motion!.cam[i * 3], motion!.cam[i * 3 + 1], motion!.cam[i * 3 + 2]);
    },

    addAnnotation: (type) => {
      const { project } = get();
      const view = get().currentView();
      if (!project || !view) {
        set({ error: "Move the playhead onto a recording to add this." });
        return;
      }
      const a = newAnnotation(project, type, get().sourceTime(), view);
      get().update((p) => {
        p.annotations.push(a);
        p.annotations.sort((x, y) => x.startMs - y.startMs);
      });
      set({ selection: { kind: "annotation", id: a.id }, playing: false });
      // Step past the fade-in so the new annotation is fully visible.
      get().seekSource(a.startMs + Math.min(250, (a.endMs - a.startMs) / 2));
    },

    editAnnotation: (id, patch, key) => {
      get().update(
        (p) => {
          const a = p.annotations.find((x) => x.id === id);
          if (a) Object.assign(a, patch);
        },
        { key: key ? `ann:${id}:${key}` : undefined },
      );
    },

    addCard: (where) => {
      const { project } = get();
      if (!project) return;
      const card = newCard(project, where === "end" ? "Thanks for watching" : where === "start" ? "Title" : "Section", where === "end" ? "" : "A short subtitle");
      const here = spanAt(project, get().sourceTime())?.index ?? 0;
      const index = where === "start" ? 0 : where === "end" ? project.clips.length : here;
      // The very first clip has nothing to fade in from.
      if (index === 0) card.transition = { type: "cut", durationMs: 500 };
      get().update((p) => {
        insertClip(p, index, card);
        const next = p.clips[index + 1];
        if (index === 0 && next && next.transition.type === "cut") next.transition = { type: "crossfade", durationMs: 500 };
      });
      set({ selection: { kind: "clip", id: card.id } });
      const start = spans(get().project!)[index]?.startMs ?? 0;
      get().seekSource(start + 600);
    },

    addRecording: async (from) => {
      const { project, projectPath } = get();
      if (!project || !projectPath) return;
      try {
        const imported = await api.importClip(projectPath, project, from);
        if (get().projectPath !== projectPath) return;
        get().update((p) => appendImported(p, imported));
        set({ selection: { kind: "clip", id: imported.clip.id } });
        await get().regenerateZooms();
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    editClip: (id, patch, key) => {
      get().update(
        (p) => {
          const i = p.clips.findIndex((c) => c.id === id);
          const c = p.clips[i];
          if (!c) return;
          // Changing a card's length moves everything after it.
          if (c.kind === "card" && "durationMs" in patch && typeof patch.durationMs === "number") {
            const end = spans(p)[i].endMs;
            const next = Math.max(300, Math.min(60_000, patch.durationMs));
            shiftFrom(p, end, next - c.durationMs);
            c.durationMs = next;
            return;
          }
          Object.assign(c, patch);
        },
        { key: key ? `clip:${id}:${key}` : undefined },
      );
    },

    removeClip: (id) => {
      const { project } = get();
      if (!project) return;
      const index = project.clips.findIndex((c) => c.id === id);
      const probe = structuredClone(project);
      if (!removeClip(probe, index)) {
        set({ error: "A project needs at least one recording." });
        return;
      }
      get().update((p) => void removeClip(p, index));
      set({ selection: null });
    },

    moveClip: (id, by) => {
      const { project } = get();
      if (!project) return;
      const from = project.clips.findIndex((c) => c.id === id);
      const to = from + by;
      if (from < 0 || to < 0 || to >= project.clips.length) return;
      get().update((p) => moveClip(p, from, to));
    },

    addWebcamKey: () => {
      const { project } = get();
      if (!project) return;
      const atMs = get().sourceTime();
      const key: WebcamKey = { id: newId("w"), atMs, ...webcamStateAt(project, atMs) };
      get().update((p) => {
        p.webcamKeys = p.webcamKeys.filter((k) => Math.abs(k.atMs - atMs) > 1);
        p.webcamKeys.push(key);
        p.webcamKeys.sort((a, b) => a.atMs - b.atMs);
      });
      set({ selection: { kind: "webcamKey", id: key.id } });
    },

    editWebcamKey: (id, patch, key) => {
      get().update(
        (p) => {
          const k = p.webcamKeys.find((x) => x.id === id);
          if (k) Object.assign(k, patch);
          p.webcamKeys.sort((a, b) => a.atMs - b.atMs);
        },
        { key: key ? `cam:${id}:${key}` : undefined },
      );
    },

    selectWords: (wordSel) => set({ wordSel }),

    cutWords: (i, j) => {
      const words = get().project?.transcript?.words;
      if (!words || !words[i] || !words[j]) return;
      const cut = wordsCut(words, Math.min(i, j), Math.max(i, j));
      get().update((p) => void (p.cuts = addCut(p.cuts, cut, totalDuration(p))));
      set({ wordSel: null });
    },

    restoreWords: (i, j) => {
      const words = get().project?.transcript?.words;
      if (!words || !words[i] || !words[j]) return;
      get().update((p) => void (p.cuts = restoreWords(p.cuts, words, Math.min(i, j), Math.max(i, j), totalDuration(p))));
      set({ wordSel: null });
    },

    editWord: (i, text) => {
      const clean = text.trim();
      get().update((p) => {
        const w = p.transcript?.words[i];
        if (w && clean) w.text = clean;
      });
    },

    transcribe: async (model, language) => {
      const { project, projectPath } = get();
      if (!project || !projectPath) return;
      set((s) => ({ jobs: { ...s.jobs, transcribe: { job: "transcribe", stage: "start", fraction: 0 } } }));
      try {
        const transcript = await api.transcribe(projectPath, project, model, language);
        if (get().projectPath !== projectPath) return;
        get().update((p) => void (p.transcript = transcript));
        void get().saveSettings({ speechModel: model });
        void api.speechModels().then((speech) => set({ speech }));
      } catch (e) {
        if (errorMessage(e) !== "cancelled") set({ error: errorMessage(e) });
      } finally {
        set((s) => ({ jobs: omit(s.jobs, "transcribe") }));
      }
    },

    applyCuts: (cuts) => {
      if (!cuts.length) return;
      get().update((p) => void (p.cuts = normalizeCuts([...p.cuts, ...cuts], totalDuration(p))));
    },

    cancelJob: (job) => void api.cancelJob(job).catch(() => undefined),

    applyStyle: (style) => get().update((p) => applyStyle(p, style)),

    saveStyle: async (name) => {
      const { project } = get();
      const clean = name.trim();
      if (!project || !clean) return;
      if (BUILTIN_STYLES.some((b) => b.name === clean)) {
        set({ error: `"${clean}" is a built-in style; pick another name.` });
        return;
      }
      const mine = [...userStyles().filter((s) => s.name !== clean), extractStyle(project, clean)];
      try {
        await api.saveStyles(mine);
        set({ styles: [...BUILTIN_STYLES, ...mine] });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    deleteStyle: async (name) => {
      const mine = userStyles().filter((s) => s.name !== name);
      try {
        await api.saveStyles(mine);
        set({ styles: [...BUILTIN_STYLES, ...mine] });
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },

    saveSettings: async (patch) => {
      const settings = { ...get().settings, ...patch };
      set({ settings });
      try {
        await api.saveSettings(settings);
      } catch (e) {
        set({ error: errorMessage(e) });
      }
    },
  };
});

function omit<T>(map: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _gone, ...rest } = map;
  return rest;
}
