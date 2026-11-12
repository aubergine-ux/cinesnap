import { useState } from "react";
import { pickImage } from "../../lib/api";
import { BUILTIN_STYLES } from "../../lib/edit";
import { useStore } from "../../lib/store";
import type { Aspect, Background, KeystrokeSettings, Project } from "../../lib/types";
import { AlphaColorInput, ColorInput, Section, Segmented, SelectRow, Slider, Toggle } from "../controls";
import { FONT_OPTIONS } from "./Inspector";

export const GRADIENTS: { from: string; to: string; angle: number }[] = [
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

function swatch(bg: Background): string {
  if (bg.type === "solid") return bg.color;
  if (bg.type === "gradient") return `linear-gradient(${bg.angle}deg, ${bg.from}, ${bg.to})`;
  return "#2a2d38";
}

function Presets() {
  const styles = useStore((s) => s.styles);
  const { applyStyle, saveStyle, deleteStyle } = useStore.getState();
  const [name, setName] = useState("");
  const builtin = (n: string) => BUILTIN_STYLES.some((b) => b.name === n);
  return (
    <Section title="Style presets">
      <div className="style-list">
        {styles.map((s) => (
          <div key={s.name} className="style-item">
            <button className="style-apply" title={`Apply "${s.name}"`} onClick={() => applyStyle(s)}>
              <span className="style-swatch" style={{ background: swatch(s.frame.background) }}>
                <i style={{ background: s.theme.accent }} />
              </span>
              <span className="style-name">{s.name}</span>
              <span className="style-font">{s.theme.font}</span>
            </button>
            {!builtin(s.name) && (
              <button className="icon-btn ghost" title="Delete preset" onClick={() => void deleteStyle(s.name)}>
                ✕
              </button>
            )}
          </div>
        ))}
      </div>
      <form
        className="file-row"
        onSubmit={(e) => {
          e.preventDefault();
          void saveStyle(name).then(() => setName(""));
        }}
      >
        <input className="text-input" placeholder="Name this look…" value={name} maxLength={40} onChange={(e) => setName(e.target.value)} />
        <button className="btn small" type="submit" disabled={!name.trim()}>
          Save
        </button>
      </form>
      <p className="hint">A preset keeps the background, padding, cursor, fonts and colours, so every video can look the same. Applying one can be undone.</p>
    </Section>
  );
}

export function StylePanel() {
  const project = useStore((s) => s.project)!;
  const cursorMode = useStore((s) => s.motion?.cursorMode ?? s.capabilities?.cursor ?? "metadata");
  const { update } = useStore.getState();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });

  const bg = project.frame.background;
  const setBg = (b: Background, key = "bg") => set(key, (p) => void (p.frame.background = b));
  const chooseImage = async (type: "image" | "wallpaper") => {
    const path = await pickImage();
    if (!path) return;
    setBg(type === "image" ? { type, path } : { type, path, blur: bg.type === "wallpaper" ? bg.blur : 40 });
  };
  const theme = project.theme;
  const keys = project.keystrokes;

  return (
    <>
      <Presets />

      <Section title="Fonts and colours">
        <SelectRow label="Font" value={theme.font} options={FONT_OPTIONS} onChange={(v) => set("theme:font", (p) => void (p.theme.font = v))} />
        <ColorInput label="Accent" value={theme.accent} onChange={(v) => set("theme:accent", (p) => void (p.theme.accent = v))} />
        <ColorInput label="Text" value={theme.text.slice(0, 7)} onChange={(v) => set("theme:text", (p) => void (p.theme.text = v))} />
        <AlphaColorInput label="Panels" value={theme.surface} onChange={(v) => set("theme:surface", (p) => void (p.theme.surface = v))} />
        <p className="hint">Used by cards, callouts, captions and the keystroke overlay.</p>
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

      <Section title="Cursor" defaultOpen={false}>
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

      <Section title="Keystrokes" defaultOpen={false}>
        <Toggle label="Show shortcuts as they are pressed" value={keys.show} onChange={(v) => set("keys:show", (p) => void (p.keystrokes.show = v))} />
        {keys.show && (
          <>
            <div className="stack">
              <span className="row-label">Position</span>
              <Segmented<KeystrokeSettings["position"]>
                value={keys.position}
                onChange={(v) => set("keys:pos", (p) => void (p.keystrokes.position = v))}
                options={[
                  { value: "bottom-left", label: "Left" },
                  { value: "bottom", label: "Bottom" },
                  { value: "bottom-right", label: "Right" },
                  { value: "top", label: "Top" },
                ]}
              />
            </div>
            <Slider label="Size" value={keys.size} min={0.5} max={2.5} step={0.05} format={(v) => `${v.toFixed(2)}×`} onChange={(v) => set("keys:size", (p) => void (p.keystrokes.size = v))} />
            <Slider label="Stay on screen" value={keys.holdMs} min={500} max={4000} step={100} format={ms} onChange={(v) => set("keys:hold", (p) => void (p.keystrokes.holdMs = v))} />
          </>
        )}
        <p className="hint">Only shortcuts and keys like Enter or Esc are ever recorded. Text you type is not, so passwords cannot end up in a video.</p>
      </Section>
    </>
  );
}
