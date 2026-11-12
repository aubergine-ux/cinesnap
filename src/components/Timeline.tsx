import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { useStore } from "../lib/store";
import { addCut, moveZoom, normalizeCuts, resizeZoom, setTrimEnd, setTrimStart, trimBounds } from "../lib/time";
import { clampZoom, formatTime, ticks, zoomAround } from "../lib/timeline";

const PAD = 14;

/** Track a pointer drag until release. */
function drag(e: RPointerEvent, onMove: (dx: number, ev: PointerEvent) => void, onEnd?: (moved: boolean) => void) {
  if (e.button !== 0) return;
  e.preventDefault();
  e.stopPropagation();
  const x0 = e.clientX;
  let moved = false;
  const move = (ev: PointerEvent) => {
    const dx = ev.clientX - x0;
    if (!moved && Math.abs(dx) < 3) return;
    moved = true;
    onMove(dx, ev);
  };
  const up = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", up);
    window.removeEventListener("pointercancel", up);
    onEnd?.(moved);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", up);
  window.addEventListener("pointercancel", up);
}

function Playhead({ pxPerMs, scroller }: { pxPerMs: number; scroller: React.RefObject<HTMLDivElement> }) {
  const src = useStore((s) => (s.motion && s.motion.frames ? s.motion.srcMs[Math.min(s.frame, s.motion.frames - 1)] : 0));
  const playing = useStore((s) => s.playing);
  const x = PAD + src * pxPerMs;
  useEffect(() => {
    const el = scroller.current;
    if (!el || !playing) return;
    if (x < el.scrollLeft + 20 || x > el.scrollLeft + el.clientWidth - 20) el.scrollLeft = Math.max(0, x - 80);
  }, [x, playing, scroller]);
  return (
    <div className="tl-playhead" style={{ transform: `translateX(${x}px)` }}>
      <span />
    </div>
  );
}

export function Timeline() {
  const project = useStore((s) => s.project)!;
  const events = useStore((s) => s.motion?.events);
  const selection = useStore((s) => s.selection);
  const cutMode = useStore((s) => s.cutMode);
  const cutMark = useStore((s) => s.cutMark);
  const past = useStore((s) => s.past.length);
  const future = useStore((s) => s.future.length);
  const { update, select, editZoom, addZoomAt, seekSource, setPlaying, setCutMode, undo, redo, sourceTime } = useStore.getState();

  const scroller = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState(800);
  const [zoom, setZoom] = useState<number | null>(null); // null = fit to width
  const [ghost, setGhost] = useState<{ a: number; b: number } | null>(null);
  const pendingScroll = useRef<number | null>(null);

  const duration = project.source.video.durationMs;
  const inner = Math.max(100, viewport - 2 * PAD);
  const fit = duration > 0 ? inner / duration : 1;
  const pxPerMs = zoom === null ? fit : clampZoom(zoom, duration, inner);
  const width = duration * pxPerMs;

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setViewport(el.clientWidth));
    ro.observe(el);
    setViewport(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  useLayoutEffect(() => {
    if (pendingScroll.current !== null && scroller.current) {
      scroller.current.scrollLeft = pendingScroll.current;
      pendingScroll.current = null;
    }
  }, [pxPerMs]);

  const zoomBy = useCallback(
    (factor: number, anchorPx: number) => {
      const el = scroller.current;
      if (!el) return;
      const next = zoomAround(pxPerMs, el.scrollLeft, anchorPx - PAD, factor, duration, inner);
      pendingScroll.current = next.scrollPx;
      setZoom(next.pxPerMs <= fit * 1.001 ? null : next.pxPerMs);
    },
    [pxPerMs, duration, inner, fit],
  );

  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      zoomBy(Math.exp(-e.deltaY * 0.0025), e.clientX - el.getBoundingClientRect().left);
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [zoomBy]);

  const msAt = (clientX: number) => {
    const el = scroller.current!;
    const x = clientX - el.getBoundingClientRect().left + el.scrollLeft - PAD;
    return Math.max(0, Math.min(duration, x / pxPerMs));
  };

  const scrub = (e: RPointerEvent) => {
    if (e.button !== 0) return;
    setPlaying(false);
    seekSource(msAt(e.clientX));
    drag(e, (_dx, ev) => seekSource(msAt(ev.clientX)));
  };

  const cuts = useMemo(() => normalizeCuts(project.cuts, duration), [project.cuts, duration]);
  const trim = trimBounds(cuts, duration);
  const marks = useMemo(() => ticks(duration, pxPerMs), [duration, pxPerMs]);

  const startCutDrag = (e: RPointerEvent) => {
    if (!cutMode) return scrub(e);
    const a = msAt(e.clientX);
    let b = a;
    drag(
      e,
      (_dx, ev) => {
        b = msAt(ev.clientX);
        setGhost({ a, b });
      },
      (moved) => {
        setGhost(null);
        if (moved && Math.abs(b - a) >= 50) {
          const cut = { startMs: Math.min(a, b), endMs: Math.max(a, b) };
          update((p) => void (p.cuts = addCut(p.cuts, cut, duration)));
        }
      },
    );
  };

  const resizeCut = (e: RPointerEvent, startMs: number, endMs: number, edge: "start" | "end") => {
    let cur = { startMs, endMs };
    select({ kind: "cut", startMs });
    drag(e, (_dx, ev) => {
      const ms = msAt(ev.clientX);
      const next = edge === "start" ? { startMs: Math.min(ms, cur.endMs - 50), endMs: cur.endMs } : { startMs: cur.startMs, endMs: Math.max(ms, cur.startMs + 50) };
      const prev = cur;
      cur = next;
      update((p) => void (p.cuts = normalizeCuts([...p.cuts.filter((c) => Math.abs(c.startMs - prev.startMs) > 0.5), next], duration)), { key: "cut-resize" });
      select({ kind: "cut", startMs: next.startMs });
    });
  };

  return (
    <div className={`timeline ${cutMode ? "cut-mode" : ""}`}>
      <div className="tl-toolbar">
        <button className="icon-btn" title="Undo (Ctrl+Z)" disabled={!past} onClick={undo}>
          ↶
        </button>
        <button className="icon-btn" title="Redo (Ctrl+Shift+Z)" disabled={!future} onClick={redo}>
          ↷
        </button>
        <span className="sep" />
        <button className={`btn small ${cutMode ? "active" : ""}`} title="Drag on the cut track to remove a section (C)" onClick={() => setCutMode(!cutMode)}>
          ✂ Cut
        </button>
        <button className="btn small" title="Add a zoom at the playhead" onClick={() => addZoomAt(sourceTime())}>
          ＋ Zoom
        </button>
        <span className="tl-hint">
          {cutMode
            ? "Drag on the cut track to remove a section"
            : cutMark !== null
              ? "Cut start marked — move the playhead and press S again"
              : "Double-click the zoom track to add a zoom · S marks a cut · Ctrl+scroll to zoom"}
        </span>
        <span className="grow" />
        <button className="icon-btn" title="Zoom out" onClick={() => zoomBy(1 / 1.5, viewport / 2)}>
          −
        </button>
        <button className="btn small" title="Fit timeline to window" onClick={() => setZoom(null)}>
          Fit
        </button>
        <button className="icon-btn" title="Zoom in" onClick={() => zoomBy(1.5, viewport / 2)}>
          +
        </button>
      </div>

      <div className="tl-scroll" ref={scroller}>
        <div className="tl-content" style={{ width: width + 2 * PAD }}>
          <div className="tl-ruler" onPointerDown={scrub}>
            {marks.map((t) => (
              <span key={t} className="tick" style={{ left: PAD + t * pxPerMs }}>
                {formatTime(t, false)}
              </span>
            ))}
          </div>

          <div className="tl-events" onPointerDown={scrub}>
            {events?.keys.map((t, i) => <i key={`k${i}`} className="ev key" style={{ left: PAD + t * pxPerMs }} />)}
            {events?.clicks.map((t, i) => <i key={`c${i}`} className="ev click" style={{ left: PAD + t * pxPerMs }} />)}
          </div>

          <div
            className="tl-track zoom-track"
            onPointerDown={(e) => {
              if (e.target === e.currentTarget) select(null);
            }}
            onDoubleClick={(e) => {
              if (e.target === e.currentTarget) addZoomAt(msAt(e.clientX));
            }}
          >
            <span className="track-label">Zoom</span>
            {project.zooms.map((z) => {
              const selected = selection?.kind === "zoom" && selection.id === z.id;
              const w = (z.endMs - z.startMs) * pxPerMs;
              return (
                <div
                  key={z.id}
                  className={`zoom-block ${selected ? "selected" : ""} ${z.manual ? "manual" : "auto"}`}
                  style={{ left: PAD + z.startMs * pxPerMs, width: w }}
                  onPointerDown={(e) => {
                    select({ kind: "zoom", id: z.id });
                    const start0 = z.startMs;
                    drag(e, (dx) => {
                      const r = moveZoom(useStore.getState().project!.zooms, z.id, start0 + dx / pxPerMs, duration);
                      if (r) editZoom(z.id, r, "move");
                    });
                  }}
                  onDoubleClick={(e) => e.stopPropagation()}
                >
                  <span
                    className="ramp in"
                    style={{ width: Math.min(w / 2, z.easeInMs * pxPerMs) }}
                  />
                  <span
                    className="ramp out"
                    style={{ width: Math.min(w / 2, z.easeOutMs * pxPerMs) }}
                  />
                  <span className="zoom-label">
                    {z.scale.toFixed(1)}× {z.manual ? "" : "· auto"}
                  </span>
                  {(["start", "end"] as const).map((edge) => (
                    <span
                      key={edge}
                      className={`handle ${edge}`}
                      onPointerDown={(e) => {
                        select({ kind: "zoom", id: z.id });
                        drag(e, (_dx, ev) => {
                          const r = resizeZoom(useStore.getState().project!.zooms, z.id, edge, msAt(ev.clientX), duration);
                          if (r) editZoom(z.id, r, "resize");
                        });
                      }}
                    />
                  ))}
                </div>
              );
            })}
          </div>

          <div className="tl-track cut-track" onPointerDown={startCutDrag}>
            <span className="track-label">Cuts</span>
            <div className="kept" style={{ left: PAD + trim.startMs * pxPerMs, width: Math.max(0, (trim.endMs - trim.startMs) * pxPerMs) }} />
            {cuts.map((c) => {
              const leading = c.startMs < 1;
              const trailing = c.endMs > duration - 1;
              const selected = selection?.kind === "cut" && Math.abs(selection.startMs - c.startMs) < 0.5;
              return (
                <div
                  key={c.startMs}
                  className={`cut-block ${selected ? "selected" : ""}`}
                  style={{ left: PAD + c.startMs * pxPerMs, width: (c.endMs - c.startMs) * pxPerMs }}
                  title="Removed section — select and press Delete to restore"
                  onPointerDown={(e) => {
                    e.stopPropagation();
                    select({ kind: "cut", startMs: c.startMs });
                  }}
                >
                  {!leading && <span className="handle start" onPointerDown={(e) => resizeCut(e, c.startMs, c.endMs, "start")} />}
                  {!trailing && <span className="handle end" onPointerDown={(e) => resizeCut(e, c.startMs, c.endMs, "end")} />}
                </div>
              );
            })}
            {ghost && <div className="cut-block ghost" style={{ left: PAD + Math.min(ghost.a, ghost.b) * pxPerMs, width: Math.abs(ghost.b - ghost.a) * pxPerMs }} />}
            <span
              className="trim start"
              title="Trim start"
              style={{ left: PAD + trim.startMs * pxPerMs }}
              onPointerDown={(e) => drag(e, (_dx, ev) => update((p) => void (p.cuts = setTrimStart(p.cuts, msAt(ev.clientX), duration)), { key: "trim-start" }))}
            />
            <span
              className="trim end"
              title="Trim end"
              style={{ left: PAD + trim.endMs * pxPerMs }}
              onPointerDown={(e) => drag(e, (_dx, ev) => update((p) => void (p.cuts = setTrimEnd(p.cuts, msAt(ev.clientX), duration)), { key: "trim-end" }))}
            />
          </div>

          {cutMark !== null && <div className="tl-mark" style={{ left: PAD + cutMark * pxPerMs }} />}
          <Playhead pxPerMs={pxPerMs} scroller={scroller} />
        </div>
      </div>
    </div>
  );
}
