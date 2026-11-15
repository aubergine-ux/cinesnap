import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { outputRatio } from "../lib/edit";
import { fitSize } from "../lib/layout";
import { useStore } from "../lib/store";
import { formatTime } from "../lib/timeline";
import { Player } from "../render/player";
import { EditOverlay } from "./EditOverlay";

function Transport() {
  const frame = useStore((s) => s.frame);
  const motion = useStore((s) => s.motion);
  const playing = useStore((s) => s.playing);
  const { setPlaying, setFrame } = useStore.getState();
  const frames = motion?.frames ?? 0;
  const fps = motion?.fps ?? 60;
  const max = Math.max(1, frames - 1);
  return (
    <div className="transport">
      <button className="play" title="Play / pause (Space)" disabled={!frames} onClick={() => setPlaying(!playing)}>
        {playing ? "❚❚" : "▶"}
      </button>
      <span className="time">
        {formatTime((frame * 1000) / fps)} <em>/ {formatTime(motion?.durationMs ?? 0)}</em>
      </span>
      <input
        className="scrub"
        type="range"
        min={0}
        max={max}
        step={1}
        value={Math.min(frame, max)}
        style={{ ["--pct" as string]: `${(Math.min(frame, max) / max) * 100}%` }}
        onChange={(e) => {
          setPlaying(false);
          setFrame(Number(e.target.value));
        }}
      />
    </div>
  );
}

export function Preview() {
  const path = useStore((s) => s.projectPath)!;
  const aspect = useStore((s) => outputRatio(s.project!));
  const ready = useStore((s) => s.motion !== null);
  const stage = useRef<HTMLDivElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [box, setBox] = useState({ w: 0, h: 0 });

  useLayoutEffect(() => {
    const el = stage.current;
    if (!el) return;
    const measure = () => setBox({ w: el.clientWidth, h: el.clientHeight });
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    measure();
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    if (!canvas.current) return;
    const player = new Player(canvas.current, path);
    return () => player.destroy();
  }, [path]);

  const size = fitSize(aspect, Math.max(0, box.w - 48), Math.max(0, box.h - 48));

  return (
    <div className="preview">
      <div className="stage" ref={stage}>
        <div className="canvas-wrap" style={{ width: Math.floor(size.width), height: Math.floor(size.height) }}>
          <canvas ref={canvas} className={ready ? "ready" : ""} style={{ width: Math.floor(size.width), height: Math.floor(size.height) }} />
          <EditOverlay width={Math.floor(size.width)} height={Math.floor(size.height)} />
        </div>
      </div>
      <Transport />
    </div>
  );
}
