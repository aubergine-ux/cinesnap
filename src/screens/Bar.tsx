import { useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { formatTime } from "../lib/timeline";
import type { Sources } from "../lib/types";
import { useRecordingControls, useRecordingStatus } from "./Recording";

/** The floating control bar shown while recording. */
export function Bar() {
  const status = useRecordingStatus();
  const [error, setError] = useState<string | null>(null);
  const { busy, pause, marker, stop, discard } = useRecordingControls(true, setError);
  const [sources, setSources] = useState<Sources | null>(null);
  const [flash, setFlash] = useState(false);

  useEffect(() => {
    void api.listSources().then(setSources, () => undefined);
    const offMarker = api.onRecordingMarker(() => {
      setFlash(true);
      window.setTimeout(() => setFlash(false), 600);
    });
    const offError = api.onRecordingError(setError);
    return () => {
      offMarker();
      offError();
    };
  }, []);

  useEffect(() => {
    if (!error) return;
    const t = window.setTimeout(() => setError(null), 6000);
    return () => window.clearTimeout(t);
  }, [error]);

  const switchTo = async (value: string) => {
    if (!value) return;
    const target = value === "portal" || value === "all" ? { kind: value as "portal" | "all" } : { kind: "screen" as const, id: value.slice(7) };
    try {
      await api.switchTarget(target);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const paused = status?.paused ?? false;
  const counting = status !== null && status.elapsedMs < status.countdownMs;
  const screens = sources?.screens ?? [];

  return (
    <div className="bar" onPointerEnter={() => api.barHover(true)} onPointerLeave={() => api.barHover(false)} data-tauri-drag-region>
      <span className={`dot${paused ? "" : " pulse"}${flash ? " flash" : ""}`} aria-hidden />
      <span className="bar-time" aria-live="polite" data-tauri-drag-region>
        {counting ? Math.ceil((status.countdownMs - status.elapsedMs) / 1000) : formatTime(Math.max(0, (status?.elapsedMs ?? 0) - (status?.countdownMs ?? 0)))}
      </span>
      <button className="icon-btn" disabled={busy || !status} onClick={() => pause(paused)} title={paused ? "Resume" : "Pause"} aria-label={paused ? "Resume" : "Pause"}>
        {paused ? "▶" : "❚❚"}
      </button>
      <button className="icon-btn" disabled={busy || paused} onClick={marker} title="Mark this moment" aria-label="Add marker">
        ⚑
      </button>
      {sources?.portal ? (
        <button className="icon-btn" disabled={busy} onClick={() => void switchTo("portal")} title="Record something else" aria-label="Switch screen">
          ⇄
        </button>
      ) : (
        screens.length > 1 && (
          <select className="bar-switch" value="" onChange={(e) => void switchTo(e.target.value)} aria-label="Switch screen" disabled={busy}>
            <option value="">Screen…</option>
            {screens.map((s) => (
              <option key={s.id} value={`screen:${s.id}`}>
                {s.name}
              </option>
            ))}
            <option value="all">All screens</option>
          </select>
        )
      )}
      <button className="btn primary small" disabled={busy} onClick={stop}>
        Stop
      </button>
      <button className="icon-btn ghost" disabled={busy} onClick={discard} title="Discard the recording" aria-label="Discard">
        ✕
      </button>
      {error && (
        <div className="bar-error" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
