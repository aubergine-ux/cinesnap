import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type PointerEvent as RPointerEvent } from "react";
import { spans, totalDuration } from "../lib/edit";
import { useStore } from "../lib/store";
import type { Annotation, AnnotationType } from "../lib/types";
import { addCut, moveZoom, normalizeCuts, resizeZoom, setTrimEnd, setTrimStart, trimBounds } from "../lib/time";
import { clampZoom, formatTime, ticks, zoomAround } from "../lib/timeline";

const PAD = 14;
const MIN_ANNOTATION_MS = 300;

const TOOLS: { type: AnnotationType; icon: string; label: string; hint: string }[] = [
  { type: "text", icon: "T", label: "Text", hint: "Add a text callout" },
  { type: "arrow", icon: "↗", label: "Arrow", hint: "Point at something" },
  { type: "box", icon: "▭", label: "Box", hint: "Outline an area" },
  { type: "highlight", icon: "▬", label: "Highlight", hint: "Mark an area like a highlighter pen" },
  { type: "spotlight", icon: "◎", label: "Spotlight", hint: "Dim everything except an area" },
  { type: "blur", icon: "▒", label: "Blur", hint: "Hide sensitive information" },
];

function annotationLabel(a: Annotation): string {
  return a.type === "text" ? a.text.split("\n")[0] || "Text" : (TOOLS.find((t) => t.type === a.type)?.label ?? a.type);
}

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
  const { update, select, editZoom, addZoomAt, seekSource, setPlaying, setCutMode, undo, redo, sourceTime, addAnnotation, editAnnotation, editWebcamKey, addWebcamKey } = useStore.getState();

  const scroller = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState(800);
  const [zoom, setZoom] = useState<number | null>(null); // null = fit to width
  const [ghost, setGhost] = useState<{ a: number; b: number } | null>(null);
  const pendingScroll = useRef<number | null>(null);

  const duration = totalDuration(project);
  const clipSpans = useMemo(() => spans(project), [project.clips]); // eslint-disable-line react-hooks/exhaustive-deps
  const hasWebcam = project.clips.some((c) => c.kind === "recording" && c.source.webcam);
  /** Limits of the clip a time falls in: zooms belong to one recording. */
  const clipBounds = (ms: number) => clipSpans.find((c) => ms >= c.startMs - 0.5 && ms < c.endMs) ?? { startMs: 0, endMs: duration };
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
        <span className="sep" />
        {TOOLS.map((t) => (
          <button key={t.type} className="btn small tool" title={`${t.hint} at the playhead`} onClick={() => addAnnotation(t.type)}>
            <span aria-hidden>{t.icon}</span> {t.label}
          </button>
        ))}
        <span className="tl-hint">
          {cutMode
            ? "Drag on the cut track to remove a section"
            : cutMark !== null
              ? "Cut start marked — move the playhead and press S again"
              : "Double-click the zoom track to add a zoom · S marks a cut"}
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

          {project.clips.length > 1 && (
            <div className="tl-track clip-track" onPointerDown={(e) => e.target === e.currentTarget && select(null)}>
              <span className="track-label">Clips</span>
              {clipSpans.map((c) => {
                const clip = project.clips[c.index];
                const selected = selection?.kind === "clip" && selection.id === clip.id;
                const tr = clip.transition.type;
                return (
                  <div
                    key={clip.id}
                    className={`clip-block ${clip.kind} ${selected ? "selected" : ""}`}
                    style={{ left: PAD + c.startMs * pxPerMs, width: Math.max(2, c.durationMs * pxPerMs - 1) }}
                    title={clip.kind === "card" ? "Card" : "Recording"}
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      select({ kind: "clip", id: clip.id });
                    }}
                  >
                    {c.index > 0 && tr !== "cut" && <span className="transition-mark" title={tr === "zoom" ? "Zoom through" : "Crossfade"}>{tr === "zoom" ? "⤢" : "⨉"}</span>}
                    <span className="zoom-label">{clip.kind === "card" ? clip.title.split("\n")[0] || "Card" : `Recording ${clipSpans.slice(0, c.index + 1).filter((x) => project.clips[x.index].kind === "recording").length}`}</span>
                  </div>
                );
              })}
            </div>
          )}

          {project.annotations.length > 0 && (
            <div className="tl-track note-track" onPointerDown={(e) => e.target === e.currentTarget && select(null)}>
              <span className="track-label">Notes</span>
              {project.annotations.map((a) => {
                const selected = selection?.kind === "annotation" && selection.id === a.id;
                return (
                  <div
                    key={a.id}
                    className={`note-block ${a.type} ${selected ? "selected" : ""}`}
                    style={{ left: PAD + a.startMs * pxPerMs, width: Math.max(4, (a.endMs - a.startMs) * pxPerMs) }}
                    onPointerDown={(e) => {
                      select({ kind: "annotation", id: a.id });
                      const len = a.endMs - a.startMs;
                      drag(
                        e,
                        (dx) => {
                          const start = Math.max(0, Math.min(duration - len, a.startMs + dx / pxPerMs));
                          editAnnotation(a.id, { startMs: start, endMs: start + len }, "time");
                        },
                        (moved) => !moved && seekSource(a.startMs + Math.min(len / 2, 250)),
                      );
                    }}
                  >
                    <span className="zoom-label">{annotationLabel(a)}</span>
                    {(["start", "end"] as const).map((edge) => (
                      <span
                        key={edge}
                        className={`handle ${edge}`}
                        onPointerDown={(e) => {
                          select({ kind: "annotation", id: a.id });
                          drag(e, (_dx, ev) => {
                            const ms = msAt(ev.clientX);
                            const cur = useStore.getState().project!.annotations.find((x) => x.id === a.id);
                            if (!cur) return;
                            editAnnotation(a.id, edge === "start" ? { startMs: Math.min(ms, cur.endMs - MIN_ANNOTATION_MS) } : { endMs: Math.max(ms, cur.startMs + MIN_ANNOTATION_MS) }, "time");
                          });
                        }}
                      />
                    ))}
                  </div>
                );
              })}
            </div>
          )}

          {hasWebcam && (
            <div
              className="tl-track cam-track"
              title="Double-click to move or hide the webcam from here on"
              onPointerDown={(e) => e.target === e.currentTarget && select(null)}
              onDoubleClick={(e) => {
                if (e.target !== e.currentTarget) return;
                seekSource(msAt(e.clientX));
                addWebcamKey();
              }}
            >
              <span className="track-label">Webcam</span>
              {project.webcamKeys.map((k) => (
                <span
                  key={k.id}
                  className={`cam-key ${k.show ? "" : "hidden"} ${selection?.kind === "webcamKey" && selection.id === k.id ? "selected" : ""}`}
                  style={{ left: PAD + k.atMs * pxPerMs }}
                  title={k.show ? "Webcam moves here" : "Webcam hides here"}
                  onPointerDown={(e) => {
                    select({ kind: "webcamKey", id: k.id });
                    drag(
                      e,
                      (_dx, ev) => editWebcamKey(k.id, { atMs: msAt(ev.clientX) }, "time"),
                      (moved) => !moved && seekSource(k.atMs + 500),
                    );
                  }}
                />
              ))}
            </div>
          )}

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
                    const bounds = clipBounds(start0);
                    drag(e, (dx) => {
                      const r = moveZoom(useStore.getState().project!.zooms, z.id, start0 + dx / pxPerMs, duration);
                      if (!r) return;
                      // Stay inside the recording the zoom belongs to.
                      const len = r.endMs - r.startMs;
                      const start = Math.max(bounds.startMs, Math.min(bounds.endMs - len, r.startMs));
                      editZoom(z.id, { startMs: start, endMs: start + len }, "move");
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
                        const bounds = clipBounds(z.startMs);
                        drag(e, (_dx, ev) => {
                          const ms = Math.max(bounds.startMs, Math.min(bounds.endMs, msAt(ev.clientX)));
                          const r = resizeZoom(useStore.getState().project!.zooms, z.id, edge, ms, duration);
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
