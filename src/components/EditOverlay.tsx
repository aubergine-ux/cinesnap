import type { PointerEvent as RPointerEvent } from "react";
import { annotationBox, moveAnnotation, webcamStateAt } from "../lib/edit";
import { mapPoint, webcamBox, type Box } from "../lib/layout";
import { useStore } from "../lib/store";
import type { Annotation } from "../lib/types";
import { annotationAlpha, frameGeometry } from "../render/renderer";

/** Track a pointer drag in both axes until release. */
function drag(e: RPointerEvent, onMove: (dx: number, dy: number) => void) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const { clientX: x0, clientY: y0 } = e;
  const move = (ev: PointerEvent) => onMove(ev.clientX - x0, ev.clientY - y0);
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
}

const MIN_SIZE = 8;

/**
 * Handles drawn over the preview: click an annotation to select it, drag to
 * move, drag a corner to resize; drag the webcam bubble to place it.
 * `width` and `height` are the size of the canvas in CSS pixels.
 */
export function EditOverlay({ width, height }: { width: number; height: number }) {
  const project = useStore((s) => s.project)!;
  const motion = useStore((s) => s.motion);
  const frame = useStore((s) => s.frame);
  const playing = useStore((s) => s.playing);
  const selection = useStore((s) => s.selection);
  const { select, editAnnotation, editWebcamKey, update, setPlaying } = useStore.getState();
  if (playing || width < 2 || !motion || !motion.frames) return null;

  const i = Math.min(frame, motion.frames - 1);
  const t = motion.srcMs[i];
  const geo = frameGeometry(width, height, project, motion, frame);
  // CSS pixels per source pixel.
  const k = geo ? geo.frame.w / geo.view.w : 1;
  const toCss = (x: number, y: number) => (geo ? mapPoint(geo.frame, geo.view, x, y) : { x: 0, y: 0 });
  const selected = selection?.kind === "annotation" ? selection.id : null;

  const body = (a: Annotation, box: Box | null) => {
    const o = box ? toCss(box.x, box.y) : a.type === "text" ? toCss(a.x, a.y) : { x: 0, y: 0 };
    const style = box ? { left: o.x, top: o.y, width: Math.max(10, box.w * k), height: Math.max(10, box.h * k) } : { left: o.x - 6, top: o.y - 6, width: 28, height: 28 };
    return (
      <div
        key={a.id}
        className={`ov-box ${a.id === selected ? "selected" : ""} ${box ? "" : "anchor"}`}
        style={style}
        title={a.id === selected ? "Drag to move" : "Click to select"}
        onPointerDown={(e) => {
          select({ kind: "annotation", id: a.id });
          setPlaying(false);
          const start = a;
          drag(e, (dx, dy) => editAnnotation(a.id, moveAnnotation(start, dx / k, dy / k), "move"));
        }}
      >
        {a.id === selected && a.type !== "text" && a.type !== "arrow" && (["nw", "ne", "sw", "se"] as const).map((c) => <span key={c} className={`ov-handle ${c}`} onPointerDown={(e) => resize(e, a, c)} />)}
      </div>
    );
  };

  const resize = (e: RPointerEvent, a: Annotation, corner: "nw" | "ne" | "sw" | "se") => {
    if (a.type === "text" || a.type === "arrow") return;
    const start = { x: a.x, y: a.y, width: a.width, height: a.height };
    drag(e, (dxCss, dyCss) => {
      const dx = dxCss / k;
      const dy = dyCss / k;
      const left = corner.endsWith("w");
      const top = corner.startsWith("n");
      const width = Math.max(MIN_SIZE, start.width + (left ? -dx : dx));
      const height = Math.max(MIN_SIZE, start.height + (top ? -dy : dy));
      editAnnotation(a.id, { x: left ? start.x + start.width - width : start.x, y: top ? start.y + start.height - height : start.y, width, height }, "resize");
    });
  };

  const arrowEnds = (a: Annotation) => {
    if (a.type !== "arrow" || a.id !== selected) return null;
    return (["1", "2"] as const).map((end) => {
      const p = toCss(a[`x${end}`], a[`y${end}`]);
      return (
        <span
          key={a.id + end}
          className="ov-handle free"
          style={{ left: p.x, top: p.y }}
          onPointerDown={(e) => {
            const [x0, y0] = [a[`x${end}`], a[`y${end}`]];
            drag(e, (dx, dy) => editAnnotation(a.id, { [`x${end}`]: x0 + dx / k, [`y${end}`]: y0 + dy / k } as Partial<Annotation>, `end${end}`));
          }}
        />
      );
    });
  };

  // The webcam bubble: drag to place. With a keyframe selected the keyframe
  // moves; otherwise the bubble's home position does.
  const alpha = motion.webcam[i * 4 + 3];
  const keyId = selection?.kind === "webcamKey" ? selection.id : null;
  const key = keyId ? project.webcamKeys.find((x) => x.id === keyId) : undefined;
  const bubble = alpha > 0.05 || key ? webcamBox(width, height, key?.x ?? motion.webcam[i * 4], key?.y ?? motion.webcam[i * 4 + 1], key?.size ?? motion.webcam[i * 4 + 2], 1) : null;
  const hasCam = project.clips.some((c) => c.kind === "recording" && c.source.webcam) && project.webcam.show;
  const dragBubble = (e: RPointerEvent) => {
    const from = key ?? webcamStateAt({ ...project, webcamKeys: [] }, 0);
    drag(e, (dx, dy) => {
      const x = Math.max(0, Math.min(1, from.x + dx / width));
      const y = Math.max(0, Math.min(1, from.y + dy / height));
      if (key) editWebcamKey(key.id, { x, y }, "drag");
      else update((p) => void Object.assign(p.webcam, { x, y }), { key: "cam:drag" });
    });
  };

  return (
    <div
      className="overlay"
      style={{ width, height }}
      onPointerDown={(e) => {
        if (e.target === e.currentTarget && selection?.kind === "annotation") select(null);
      }}
    >
      {geo &&
        project.annotations
          .filter((a) => annotationAlpha(a, t) > 0 || a.id === selected)
          .flatMap((a) => [body(a, annotationBox(a)), arrowEnds(a)])}
      {hasCam && bubble && (key || project.webcamKeys.length === 0 || alpha > 0.05) && (
        <div
          className={`ov-cam ${project.webcam.shape} ${key ? "selected" : ""}`}
          style={{ left: bubble.x, top: bubble.y, width: bubble.w, height: bubble.h }}
          title={key ? "Drag to move the webcam from this keyframe on" : project.webcamKeys.length ? "Select a keyframe on the timeline to move the webcam" : "Drag to place the webcam"}
          onPointerDown={key || project.webcamKeys.length === 0 ? dragBubble : undefined}
        />
      )}
    </div>
  );
}
