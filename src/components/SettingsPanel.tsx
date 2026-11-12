import { useRef } from "react";
import { pickImage } from "../lib/api";
import { GIF_PRESETS, outputFps, outputSize } from "../lib/layout";
import { useStore } from "../lib/store";
import type { Aspect, Background, Corner, Easing, ExportFormat, Project, Quality, ZoomMode } from "../lib/types";
import { ColorInput, Section, Segmented, Slider, Toggle } from "./controls";

const GRADIENTS: { from: string; to: string; angle: number }[] = [
  { from: "#6d5dfc", to: "#e44d8a", angle: 135 },
  { from: "#0ea5e9", to: "#6366f1", angle: 135 },
  { from: "#f97316", to: "#ec4899", angle: 135 },
  { from: "#10b981", to: "#0ea5e9", angle: 135 },
  { from: "#fde68a", to: "#f472b6", angle: 160 },
  { from: "#1e293b", to: "#0f172a", angle: 180 },
  { from: "#a78bfa", to: "#22d3ee", angle: 120 },
  { from: "#f43f5e", to: "#7c3aed", angle: 200 },
];

const pct = (v: number) => `${Math.round(v * 100)}%`;
const ms = (v: number) => `${(v / 1000).toFixed(2)}s`;

export function SettingsPanel({ onExport }: { onExport: () => void }) {
  const project = useStore((s) => s.project)!;
  const selection = useStore((s) => s.selection);
  const cursorMode = useStore((s) => s.motion?.cursorMode ?? s.capabilities?.cursor ?? "metadata");
  const { update, editZoom, deleteSelection, regenerateZooms } = useStore.getState();
  const regenTimer = useRef<ReturnType<typeof setTimeout>>();

  const zoom = selection?.kind === "zoom" ? project.zooms.find((z) => z.id === selection.id) : undefined;
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });

  const setAuto = (key: string, fn: (p: Project) => void) => {
    set(`auto:${key}`, fn);
    clearTimeout(regenTimer.current);
    regenTimer.current = setTimeout(() => void regenerateZooms(), 150);
  };

  const bg = project.frame.background;
  const setBg = (b: Background, key = "bg") => set(key, (p) => void (p.frame.background = b));
  const chooseImage = async (type: "image" | "wallpaper") => {
    const path = await pickImage();
    if (!path) return;
    setBg(type === "image" ? { type, path } : { type, path, blur: bg.type === "wallpaper" ? bg.blur : 40 });
  };

  const exp = project.export;
  const size = outputSize(project.frame.aspect, project.crop, exp);
  const hasWebcam = Boolean(project.source.webcam);

  return (
    <aside className="panel">
      {zoom && (
        <Section
          title="Selected zoom"
          right={
            <button className="icon-btn ghost" title="Delete zoom (Del)" onClick={deleteSelection}>
              ✕
            </button>
          }
        >
          <Slider label="Zoom level" value={zoom.scale} min={1.1} max={4} step={0.05} format={(v) => `${v.toFixed(2)}×`} onChange={(v) => editZoom(zoom.id, { scale: v }, "scale")} />
          <Slider label="Ease in" value={zoom.easeInMs} min={100} max={2000} step={50} format={ms} onChange={(v) => editZoom(zoom.id, { easeInMs: v }, "in")} />
          <Slider label="Ease out" value={zoom.easeOutMs} min={100} max={2000} step={50} format={ms} onChange={(v) => editZoom(zoom.id, { easeOutMs: v }, "out")} />
          <div className="stack">
            <span className="row-label">Easing</span>
            <Segmented<Easing>
              value={zoom.easing}
              onChange={(v) => editZoom(zoom.id, { easing: v })}
              options={[
                { value: "smooth", label: "Smooth" },
                { value: "snappy", label: "Snappy" },
                { value: "linear", label: "Linear" },
              ]}
            />
          </div>
          <div className="stack">
            <span className="row-label">Camera</span>
            <Segmented<ZoomMode>
              value={zoom.mode}
              onChange={(v) => editZoom(zoom.id, { mode: v })}
              options={[
                { value: "follow", label: "Follow cursor" },
                { value: "fixed", label: "Fixed" },
              ]}
            />
          </div>
          {zoom.mode === "fixed" && (
            <>
              <Slider label="Focus X" value={zoom.x} min={project.crop.x} max={project.crop.x + project.crop.width} step={1} format={(v) => `${Math.round(v)}px`} onChange={(v) => editZoom(zoom.id, { x: v }, "x")} />
              <Slider label="Focus Y" value={zoom.y} min={project.crop.y} max={project.crop.y + project.crop.height} step={1} format={(v) => `${Math.round(v)}px`} onChange={(v) => editZoom(zoom.id, { y: v }, "y")} />
            </>
          )}
        </Section>
      )}

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

      <Section title="Cursor">
        {cursorMode !== "metadata" && <p className="hint warn">The cursor is baked into this recording, so it cannot be restyled.</p>}
        <Toggle label="Show cursor" value={project.cursor.show} onChange={(v) => set("cursor:show", (p) => void (p.cursor.show = v))} />
        <Slider label="Size" value={project.cursor.size} min={0.6} max={4} step={0.05} format={(v) => `${v.toFixed(2)}×`} onChange={(v) => set("cursor:size", (p) => void (p.cursor.size = v))} />
        <Slider label="Smoothing" value={project.cursor.smoothing} min={0} max={1} format={pct} onChange={(v) => set("cursor:smooth", (p) => void (p.cursor.smoothing = v))} />
        <Toggle label="Click ripples" value={project.cursor.ripples} onChange={(v) => set("cursor:ripples", (p) => void (p.cursor.ripples = v))} />
        {project.cursor.ripples && <ColorInput label="Ripple colour" value={project.cursor.rippleColor} onChange={(v) => set("cursor:color", (p) => void (p.cursor.rippleColor = v))} />}
        <Slider
          label="Hide when idle"
          value={project.cursor.hideWhenIdleMs}
          min={0}
          max={5000}
          step={250}
          format={(v) => (v === 0 ? "Never" : ms(v))}
          onChange={(v) => set("cursor:idle", (p) => void (p.cursor.hideWhenIdleMs = v))}
        />
      </Section>

      <Section title="Frame">
        <div className="stack">
          <span className="row-label">Aspect ratio</span>
          <Segmented<Aspect>
            value={project.frame.aspect}
            onChange={(v) => set("aspect", (p) => void (p.frame.aspect = v))}
            options={[
              { value: "16:9", label: "16:9" },
              { value: "9:16", label: "9:16" },
              { value: "1:1", label: "1:1" },
              { value: "4:3", label: "4:3" },
              { value: "source", label: "Source" },
            ]}
          />
        </div>
        <Slider label="Padding" value={project.frame.padding} min={0} max={0.25} step={0.005} format={pct} onChange={(v) => set("padding", (p) => void (p.frame.padding = v))} />
        <Slider label="Rounded corners" value={project.frame.radius} min={0} max={60} step={1} format={(v) => `${v}px`} onChange={(v) => set("radius", (p) => void (p.frame.radius = v))} />
        <Slider label="Shadow" value={project.frame.shadow} min={0} max={1} format={pct} onChange={(v) => set("shadow", (p) => void (p.frame.shadow = v))} />

        <div className="stack">
          <span className="row-label">Background</span>
          <Segmented<Background["type"]>
            value={bg.type}
            onChange={(t) => {
              if (t === bg.type) return;
              if (t === "gradient") setBg({ type: "gradient", ...GRADIENTS[0] });
              else if (t === "solid") setBg({ type: "solid", color: "#15161c" });
              else void chooseImage(t);
            }}
            options={[
              { value: "gradient", label: "Gradient" },
              { value: "solid", label: "Solid" },
              { value: "image", label: "Image" },
              { value: "wallpaper", label: "Blurred" },
            ]}
          />
        </div>
        {bg.type === "gradient" && (
          <>
            <div className="swatches">
              {GRADIENTS.map((g) => (
                <button
                  key={g.from + g.to}
                  className={`swatch ${g.from === bg.from && g.to === bg.to ? "active" : ""}`}
                  style={{ background: `linear-gradient(${g.angle}deg, ${g.from}, ${g.to})` }}
                  title={`${g.from} → ${g.to}`}
                  onClick={() => setBg({ type: "gradient", ...g })}
                />
              ))}
            </div>
            <ColorInput label="From" value={bg.from} onChange={(v) => setBg({ ...bg, from: v }, "bg:from")} />
            <ColorInput label="To" value={bg.to} onChange={(v) => setBg({ ...bg, to: v }, "bg:to")} />
            <Slider label="Angle" value={bg.angle} min={0} max={360} step={5} format={(v) => `${v}°`} onChange={(v) => setBg({ ...bg, angle: v }, "bg:angle")} />
          </>
        )}
        {bg.type === "solid" && <ColorInput label="Colour" value={bg.color} onChange={(v) => setBg({ type: "solid", color: v }, "bg:solid")} />}
        {(bg.type === "image" || bg.type === "wallpaper") && (
          <>
            <div className="file-row">
              <code title={bg.path}>{bg.path.split(/[\\/]/).pop()}</code>
              <button className="btn small" onClick={() => void chooseImage(bg.type)}>
                Choose…
              </button>
            </div>
            {bg.type === "wallpaper" && <Slider label="Blur" value={bg.blur} min={0} max={120} step={1} format={(v) => `${v}px`} onChange={(v) => setBg({ ...bg, blur: v }, "bg:blur")} />}
          </>
        )}
      </Section>

      {hasWebcam && (
        <Section title="Webcam" defaultOpen={false}>
          <Toggle label="Show webcam" value={project.webcam.show} onChange={(v) => set("cam:show", (p) => void (p.webcam.show = v))} />
          <Slider label="Size" value={project.webcam.size} min={0.08} max={0.4} step={0.01} format={pct} onChange={(v) => set("cam:size", (p) => void (p.webcam.size = v))} />
          <div className="stack">
            <span className="row-label">Shape</span>
            <Segmented
              value={project.webcam.shape}
              onChange={(v) => set("cam:shape", (p) => void (p.webcam.shape = v))}
              options={[
                { value: "circle", label: "Circle" },
                { value: "rounded", label: "Rounded" },
              ]}
            />
          </div>
          <div className="stack">
            <span className="row-label">Position</span>
            <div className="corners">
              {(["top-left", "top-right", "bottom-left", "bottom-right"] as Corner[]).map((c) => (
                <button key={c} className={project.webcam.corner === c ? "active" : ""} title={c.replace("-", " ")} onClick={() => set("cam:corner", (p) => void (p.webcam.corner = c))} />
              ))}
            </div>
          </div>
        </Section>
      )}

      <Section title="Export">
        <div className="stack">
          <span className="row-label">Format</span>
          <Segmented<ExportFormat>
            value={exp.format}
            onChange={(v) =>
              set("export:format", (p) => {
                p.export.format = v;
                p.export.preset = v === "gif" ? "readme" : "1080p";
              })
            }
            options={[
              { value: "mp4", label: "MP4" },
              { value: "gif", label: "GIF" },
            ]}
          />
        </div>
        <div className="stack">
          <span className="row-label">Size</span>
          <Segmented<string>
            value={exp.preset}
            onChange={(v) => set("export:preset", (p) => void (p.export.preset = v))}
            options={
              exp.format === "gif"
                ? [
                    { value: "readme", label: "README · 800" },
                    { value: "small", label: "Small · 480" },
                  ]
                : [
                    { value: "720p", label: "720p" },
                    { value: "1080p", label: "1080p" },
                    { value: "1440p", label: "1440p" },
                    { value: "4k", label: "4K" },
                  ]
            }
          />
        </div>
        {exp.format === "mp4" ? (
          <div className="stack">
            <span className="row-label">Frame rate</span>
            <Segmented<number>
              value={exp.fps}
              onChange={(v) => set("export:fps", (p) => void (p.export.fps = v))}
              options={[
                { value: 30, label: "30 fps" },
                { value: 60, label: "60 fps" },
              ]}
            />
          </div>
        ) : (
          <p className="hint">{(GIF_PRESETS[exp.preset] ?? GIF_PRESETS.readme).fps} fps, optimised palette.</p>
        )}
        <div className="stack">
          <span className="row-label">Quality</span>
          <Segmented<Quality>
            value={exp.quality}
            onChange={(v) => set("export:quality", (p) => void (p.export.quality = v))}
            options={[
              { value: "low", label: "Small file" },
              { value: "medium", label: "Balanced" },
              { value: "high", label: "Best" },
            ]}
          />
        </div>
        <p className="hint">
          {size.width}×{size.height} · {outputFps(exp)} fps
        </p>
        <button className="btn primary wide" onClick={onExport}>
          Export {exp.format.toUpperCase()}
        </button>
      </Section>
    </aside>
  );
}
