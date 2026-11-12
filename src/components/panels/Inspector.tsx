import { spans } from "../../lib/edit";
import { useStore } from "../../lib/store";
import type { Annotation, Background, CardClip, Clip, Easing, TransitionKind, ZoomMode } from "../../lib/types";
import { FAMILIES } from "../../render/text";
import { AlphaColorInput, ColorInput, Section, Segmented, SelectRow, Slider, TextArea, Toggle } from "../controls";

const ms = (v: number) => `${(v / 1000).toFixed(2)}s`;
const px = (v: number) => `${Math.round(v)}px`;
const pct = (v: number) => `${Math.round(v * 100)}%`;

const NAMES: Record<Annotation["type"], string> = { text: "Text", arrow: "Arrow", box: "Box", highlight: "Highlight", spotlight: "Spotlight", blur: "Blur" };

export const FONT_OPTIONS = FAMILIES.map((f) => ({ value: f as string, label: f as string }));
const THEME_FONT = "";

function Remove({ title }: { title: string }) {
  return (
    <button className="icon-btn ghost" title={`${title} (Del)`} onClick={() => useStore.getState().deleteSelection()}>
      ✕
    </button>
  );
}

function AnnotationProps({ a }: { a: Annotation }) {
  const edit = (patch: Partial<Annotation>, key?: string) => useStore.getState().editAnnotation(a.id, patch, key);
  return (
    <Section title={NAMES[a.type]} right={<Remove title="Delete" />}>
      {a.type === "text" && (
        <>
          <TextArea label="Text" value={a.text} rows={2} onChange={(text) => edit({ text }, "text")} />
          <Slider label="Size" value={a.size} min={16} max={160} step={1} format={px} onChange={(size) => edit({ size }, "size")} />
          <SelectRow label="Font" value={a.font ?? THEME_FONT} options={[{ value: THEME_FONT, label: "Theme font" }, ...FONT_OPTIONS]} onChange={(font) => edit({ font: font || undefined })} />
          <Toggle label="Bold" value={a.bold} onChange={(bold) => edit({ bold })} />
          <ColorInput label="Colour" value={a.color} onChange={(color) => edit({ color }, "color")} />
          <Toggle label="Background" value={Boolean(a.background)} onChange={(on) => edit({ background: on ? "#14151af0" : undefined })} />
          {a.background && <AlphaColorInput label="Fill" value={a.background} onChange={(background) => edit({ background }, "bg")} />}
        </>
      )}
      {a.type === "arrow" && (
        <>
          <ColorInput label="Colour" value={a.color} onChange={(color) => edit({ color }, "color")} />
          <Slider label="Thickness" value={a.width} min={2} max={30} step={1} format={px} onChange={(width) => edit({ width }, "width")} />
        </>
      )}
      {a.type === "box" && (
        <>
          <ColorInput label="Colour" value={a.color} onChange={(color) => edit({ color }, "color")} />
          <Slider label="Thickness" value={a.stroke} min={1} max={24} step={1} format={px} onChange={(stroke) => edit({ stroke }, "stroke")} />
          <Slider label="Rounded corners" value={a.radius} min={0} max={80} step={1} format={px} onChange={(radius) => edit({ radius }, "radius")} />
        </>
      )}
      {a.type === "highlight" && (
        <>
          <ColorInput label="Colour" value={a.color} onChange={(color) => edit({ color }, "color")} />
          <Slider label="Strength" value={a.opacity} min={0.1} max={1} format={pct} onChange={(opacity) => edit({ opacity }, "opacity")} />
        </>
      )}
      {a.type === "spotlight" && (
        <>
          <Slider label="Dim the rest" value={a.dim} min={0.1} max={1} format={pct} onChange={(dim) => edit({ dim }, "dim")} />
          <div className="stack">
            <span className="row-label">Shape</span>
            <Segmented
              value={a.ellipse ? "ellipse" : "rect"}
              onChange={(v) => edit({ ellipse: v === "ellipse" })}
              options={[
                { value: "rect", label: "Rectangle" },
                { value: "ellipse", label: "Ellipse" },
              ]}
            />
          </div>
        </>
      )}
      {a.type === "blur" && (
        <>
          <Slider label="Blur" value={a.strength} min={4} max={60} step={1} format={px} onChange={(strength) => edit({ strength }, "strength")} />
          <p className="hint">The blur follows the recording when the camera zooms, and is baked into the export.</p>
        </>
      )}
      <p className="hint">Drag it in the preview to move it; drag the corners to resize. Its block on the timeline sets when it shows.</p>
    </Section>
  );
}

function TransitionProps({ clip, first }: { clip: Clip; first: boolean }) {
  const { editClip } = useStore.getState();
  if (first) return <p className="hint">The first clip has nothing to transition from.</p>;
  return (
    <>
      <div className="stack">
        <span className="row-label">Transition in</span>
        <Segmented<TransitionKind>
          value={clip.transition.type}
          onChange={(type) => editClip(clip.id, { transition: { ...clip.transition, type } })}
          options={[
            { value: "cut", label: "Cut" },
            { value: "crossfade", label: "Crossfade" },
            { value: "zoom", label: "Zoom through" },
          ]}
        />
      </div>
      {clip.transition.type !== "cut" && (
        <Slider label="Length" value={clip.transition.durationMs} min={200} max={2000} step={50} format={ms} onChange={(durationMs) => editClip(clip.id, { transition: { ...clip.transition, durationMs } }, "tr")} />
      )}
    </>
  );
}

function ClipActions({ clip, index, count }: { clip: Clip; index: number; count: number }) {
  const { moveClip } = useStore.getState();
  return (
    <div className="btn-row">
      <button className="btn small" disabled={index === 0} onClick={() => moveClip(clip.id, -1)}>
        ← Earlier
      </button>
      <button className="btn small" disabled={index === count - 1} onClick={() => moveClip(clip.id, 1)}>
        Later →
      </button>
    </div>
  );
}

function CardProps({ card, index, count }: { card: CardClip; index: number; count: number }) {
  const project = useStore((s) => s.project)!;
  const edit = (patch: Partial<CardClip>, key?: string) => useStore.getState().editClip(card.id, patch, key);
  const bg = card.background;
  const setBg = (background: Background | undefined, key?: string) => edit({ background }, key);
  return (
    <Section title="Card" right={<Remove title="Remove card" />}>
      <TextArea label="Title" value={card.title} onChange={(title) => edit({ title }, "title")} />
      <TextArea label="Subtitle" value={card.subtitle} onChange={(subtitle) => edit({ subtitle }, "subtitle")} />
      <Slider label="Length" value={card.durationMs} min={500} max={15000} step={100} format={ms} onChange={(durationMs) => edit({ durationMs }, "dur")} />
      <Slider label="Title size" value={card.titleSize} min={32} max={220} step={2} format={px} onChange={(titleSize) => edit({ titleSize }, "size")} />
      <SelectRow label="Font" value={card.font ?? THEME_FONT} options={[{ value: THEME_FONT, label: `Theme font (${project.theme.font})` }, ...FONT_OPTIONS]} onChange={(font) => edit({ font: font || undefined })} />
      <div className="stack">
        <span className="row-label">Alignment</span>
        <Segmented
          value={card.align}
          onChange={(align) => edit({ align })}
          options={[
            { value: "center", label: "Centre" },
            { value: "left", label: "Left" },
          ]}
        />
      </div>
      <ColorInput label="Title colour" value={card.titleColor.slice(0, 7)} onChange={(titleColor) => edit({ titleColor }, "tc")} />
      <AlphaColorInput label="Subtitle" value={card.subtitleColor} onChange={(subtitleColor) => edit({ subtitleColor }, "sc")} />
      <div className="stack">
        <span className="row-label">Background</span>
        <Segmented
          value={!bg ? "project" : bg.type === "gradient" ? "gradient" : "solid"}
          onChange={(v) => setBg(v === "project" ? undefined : v === "solid" ? { type: "solid", color: "#101116" } : { type: "gradient", from: project.theme.accent, to: "#101116", angle: 135 })}
          options={[
            { value: "project", label: "Same as video" },
            { value: "solid", label: "Solid" },
            { value: "gradient", label: "Gradient" },
          ]}
        />
      </div>
      {bg?.type === "solid" && <ColorInput label="Colour" value={bg.color} onChange={(color) => setBg({ type: "solid", color }, "bg")} />}
      {bg?.type === "gradient" && (
        <>
          <ColorInput label="From" value={bg.from} onChange={(from) => setBg({ ...bg, from }, "bgf")} />
          <ColorInput label="To" value={bg.to} onChange={(to) => setBg({ ...bg, to }, "bgt")} />
          <Slider label="Angle" value={bg.angle} min={0} max={360} step={5} format={(v) => `${v}°`} onChange={(angle) => setBg({ ...bg, angle }, "bga")} />
        </>
      )}
      <TransitionProps clip={card} first={index === 0} />
      <ClipActions clip={card} index={index} count={count} />
    </Section>
  );
}

/** Settings of whatever is selected on the timeline. */
export function Inspector() {
  const project = useStore((s) => s.project)!;
  const selection = useStore((s) => s.selection);
  const { editZoom, editWebcamKey } = useStore.getState();
  if (!selection) return null;

  if (selection.kind === "zoom") {
    const zoom = project.zooms.find((z) => z.id === selection.id);
    if (!zoom) return null;
    const span = spans(project).find((s) => zoom.startMs >= s.startMs - 0.5 && zoom.startMs < s.endMs);
    const clip = span && project.clips[span.index];
    const crop = clip?.kind === "recording" ? clip.crop : project.crop;
    return (
      <Section title="Selected zoom" right={<Remove title="Delete zoom" />}>
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
            <Slider label="Focus X" value={zoom.x} min={crop.x} max={crop.x + crop.width} step={1} format={px} onChange={(v) => editZoom(zoom.id, { x: v }, "x")} />
            <Slider label="Focus Y" value={zoom.y} min={crop.y} max={crop.y + crop.height} step={1} format={px} onChange={(v) => editZoom(zoom.id, { y: v }, "y")} />
          </>
        )}
      </Section>
    );
  }

  if (selection.kind === "annotation") {
    const a = project.annotations.find((x) => x.id === selection.id);
    return a ? <AnnotationProps a={a} /> : null;
  }

  if (selection.kind === "clip") {
    const index = project.clips.findIndex((c) => c.id === selection.id);
    const clip = project.clips[index];
    if (!clip) return null;
    if (clip.kind === "card") return <CardProps card={clip} index={index} count={project.clips.length} />;
    const recordings = project.clips.filter((c) => c.kind === "recording").length;
    return (
      <Section title="Recording" right={recordings > 1 ? <Remove title="Remove from the project" /> : undefined}>
        <p className="hint">
          {clip.source.video.width}×{clip.source.video.height} · {ms(clip.source.video.durationMs)}
          {clip.source.mic ? " · microphone" : ""}
          {clip.source.webcam ? " · webcam" : ""}
        </p>
        <TransitionProps clip={clip} first={index === 0} />
        <ClipActions clip={clip} index={index} count={project.clips.length} />
      </Section>
    );
  }

  if (selection.kind === "webcamKey") {
    const k = project.webcamKeys.find((x) => x.id === selection.id);
    if (!k) return null;
    return (
      <Section title="Webcam keyframe" right={<Remove title="Delete keyframe" />}>
        <Toggle label="Show webcam from here" value={k.show} onChange={(show) => editWebcamKey(k.id, { show })} />
        {k.show && (
          <>
            <Slider label="Size" value={k.size} min={0.08} max={0.6} step={0.01} format={pct} onChange={(size) => editWebcamKey(k.id, { size }, "size")} />
            <Slider label="Across" value={k.x} min={0} max={1} step={0.005} format={pct} onChange={(x) => editWebcamKey(k.id, { x }, "x")} />
            <Slider label="Down" value={k.y} min={0} max={1} step={0.005} format={pct} onChange={(y) => editWebcamKey(k.id, { y }, "y")} />
            <p className="hint">Or drag the bubble in the preview. It glides here from wherever it was.</p>
          </>
        )}
      </Section>
    );
  }
  return null;
}
