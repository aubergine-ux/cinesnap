import { useRef } from "react";
import { pickRecording } from "../../lib/api";
import { clipDuration, outputRatio, spanAt, spans } from "../../lib/edit";
import { cornerCentre } from "../../lib/layout";
import { useStore } from "../../lib/store";
import { formatTime } from "../../lib/timeline";
import type { Corner, Project, Rect, WebcamSettings } from "../../lib/types";
import { Section, Segmented, Slider, Toggle } from "../controls";

/** Smallest crop side, in source pixels. */
const MIN_CROP = 64;

const px = (v: number) => `${Math.round(v)}px`;
const pct = (v: number) => `${Math.round(v * 100)}%`;
const ms = (v: number) => `${(v / 1000).toFixed(2)}s`;

function Clips() {
  const project = useStore((s) => s.project)!;
  const selection = useStore((s) => s.selection);
  const { select, seekSource, addCard, addRecording, moveClip } = useStore.getState();
  const all = spans(project);
  return (
    <Section title="Clips">
      <ol className="clip-list">
        {project.clips.map((c, i) => (
          <li key={c.id} className={selection?.kind === "clip" && selection.id === c.id ? "selected" : ""}>
            <button
              className="clip-row"
              onClick={() => {
                select({ kind: "clip", id: c.id });
                seekSource(all[i].startMs + 1);
              }}
            >
              <span className={`clip-kind ${c.kind}`}>{c.kind === "card" ? "T" : "▶"}</span>
              <span className="clip-name">{c.kind === "card" ? c.title.split("\n")[0] || "Card" : `Recording ${project.clips.slice(0, i + 1).filter((x) => x.kind === "recording").length}`}</span>
              <span className="clip-len">{formatTime(clipDuration(c), false)}</span>
            </button>
            <button className="icon-btn ghost" title="Move earlier" disabled={i === 0} onClick={() => moveClip(c.id, -1)}>
              ↑
            </button>
            <button className="icon-btn ghost" title="Move later" disabled={i === project.clips.length - 1} onClick={() => moveClip(c.id, 1)}>
              ↓
            </button>
          </li>
        ))}
      </ol>
      <div className="btn-row">
        <button
          className="btn small"
          onClick={async () => {
            const from = await pickRecording();
            if (from) await addRecording(from);
          }}
        >
          ＋ Recording…
        </button>
        <button className="btn small" onClick={() => addCard("start")}>
          ＋ Title
        </button>
        <button className="btn small" onClick={() => addCard("here")}>
          ＋ Section
        </button>
        <button className="btn small" onClick={() => addCard("end")}>
          ＋ Outro
        </button>
      </div>
      <p className="hint">Add another .cinesnap recording to continue the video. Select a clip to choose how it comes in.</p>
    </Section>
  );
}

function Webcam() {
  const project = useStore((s) => s.project)!;
  const keys = project.webcamKeys.length;
  const { update, addWebcamKey } = useStore.getState();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });
  const w = project.webcam;
  const ratio = outputRatio(project);
  const placed = w.x !== undefined && w.y !== undefined;
  const corner = (c: Corner) => set("cam:corner", (p) => void Object.assign(p.webcam, { corner: c, ...cornerCentre(c, p.webcam.size, ratio) }));
  return (
    <Section title="Webcam" defaultOpen={false}>
      <Toggle label="Show webcam" value={w.show} onChange={(v) => set("cam:show", (p) => void (p.webcam.show = v))} />
      <Slider label="Size" value={w.size} min={0.08} max={0.6} step={0.01} format={pct} onChange={(v) => set("cam:size", (p) => void (p.webcam.size = v))} />
      <div className="stack">
        <span className="row-label">Shape</span>
        <Segmented<WebcamSettings["shape"]>
          value={w.shape}
          onChange={(v) => set("cam:shape", (p) => void (p.webcam.shape = v))}
          options={[
            { value: "circle", label: "Circle" },
            { value: "rounded", label: "Rounded" },
            { value: "square", label: "Square" },
          ]}
        />
      </div>
      <div className="stack">
        <span className="row-label">Position</span>
        <div className="corners">
          {(["top-left", "top-right", "bottom-left", "bottom-right"] as Corner[]).map((c) => (
            <button key={c} className={!placed && w.corner === c ? "active" : ""} title={c.replace("-", " ")} onClick={() => corner(c)} />
          ))}
        </div>
      </div>
      <button className="btn wide" onClick={addWebcamKey}>
        Move or hide it from the playhead on…
      </button>
      <p className="hint">
        Drag the bubble in the preview to place it anywhere. A keyframe changes where it is, how big it is or whether it shows from that moment on
        {keys ? ` (${keys} on the timeline).` : "."}
      </p>
    </Section>
  );
}

export function VideoPanel() {
  const project = useStore((s) => s.project)!;
  // The crop being edited is that of the recording under the playhead.
  const clipId = useStore((s) => {
    const p = s.project!;
    const span = spanAt(p, s.sourceTime());
    const c = span && p.clips[span.index];
    return c?.kind === "recording" ? c.id : (p.clips.find((x) => x.kind === "recording")?.id ?? "");
  });
  const { update, regenerateZooms } = useStore.getState();
  const regenTimer = useRef<ReturnType<typeof setTimeout>>();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });
  const setAuto = (key: string, fn: (p: Project) => void) => {
    set(`auto:${key}`, fn);
    clearTimeout(regenTimer.current);
    regenTimer.current = setTimeout(() => void regenerateZooms(), 150);
  };

  const clip = project.clips.find((c) => c.id === clipId);
  const hasWebcam = project.clips.some((c) => c.kind === "recording" && c.source.webcam);
  const recordings = project.clips.filter((c) => c.kind === "recording").length;

  return (
    <>
      <Clips />

      <Section title="Auto-zoom">
        <Toggle label="Zoom on clicks" value={project.autoZoom.enabled} onChange={(v) => setAuto("enabled", (p) => void (p.autoZoom.enabled = v))} />
        <Slider label="Zoom level" value={project.autoZoom.scale} min={1.2} max={4} step={0.05} format={(v) => `${v.toFixed(2)}×`} disabled={!project.autoZoom.enabled} onChange={(v) => setAuto("scale", (p) => void (p.autoZoom.scale = v))} />
        <Slider label="Merge clicks within" value={project.autoZoom.mergeGapMs} min={500} max={6000} step={100} format={ms} disabled={!project.autoZoom.enabled} onChange={(v) => setAuto("gap", (p) => void (p.autoZoom.mergeGapMs = v))} />
        <Toggle label="Zoom while typing" value={project.autoZoom.onTyping} onChange={(v) => setAuto("typing", (p) => void (p.autoZoom.onTyping = v))} />
        <button className="btn wide" onClick={() => void regenerateZooms()}>
          Regenerate zooms
        </button>
        <p className="hint">Zooms you have edited or added by hand are kept.</p>
      </Section>

      {clip?.kind === "recording" &&
        (() => {
          const video = clip.source.video;
          const crop = clip.crop;
          const cropped = crop.width < video.width || crop.height < video.height;
          const setCrop = (patch: Partial<Rect>) =>
            set(`crop:${clip.id}`, (p) => {
              const c = p.clips.find((x) => x.id === clip.id);
              if (c?.kind === "recording") Object.assign(c.crop, patch);
            });
          return (
            <Section title={recordings > 1 ? "Crop (recording at the playhead)" : "Crop"} defaultOpen={cropped}>
              <Slider label="Left" value={crop.x} min={0} max={video.width - MIN_CROP} step={2} format={px} onChange={(v) => setCrop({ x: v, width: Math.min(crop.width, video.width - v) })} />
              <Slider label="Top" value={crop.y} min={0} max={video.height - MIN_CROP} step={2} format={px} onChange={(v) => setCrop({ y: v, height: Math.min(crop.height, video.height - v) })} />
              <Slider label="Width" value={crop.width} min={MIN_CROP} max={video.width - crop.x} step={2} format={px} onChange={(v) => setCrop({ width: v })} />
              <Slider label="Height" value={crop.height} min={MIN_CROP} max={video.height - crop.y} step={2} format={px} onChange={(v) => setCrop({ height: v })} />
              <button className="btn wide" disabled={!cropped} onClick={() => setCrop({ x: 0, y: 0, width: video.width, height: video.height })}>
                Use the whole recording
              </button>
              <p className="hint">Nothing is thrown away: the full recording stays in the file, so the crop can be changed at any time.</p>
            </Section>
          );
        })()}

      {hasWebcam && <Webcam />}
    </>
  );
}
