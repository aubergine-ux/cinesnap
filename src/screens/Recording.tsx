import { useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { DEFAULT_HOTKEYS, hotkeyLabel } from "../lib/hotkeys";
import { useStore } from "../lib/store";
import { formatTime } from "../lib/timeline";
import type { RecordingStatus } from "../lib/types";

/** Live state of the recording, from the backend's ticks. */
export function useRecordingStatus(): RecordingStatus | null {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  useEffect(() => {
    void api.recordingStatus().then(setStatus, () => undefined);
    return api.onRecordingTick(setStatus);
  }, []);
  return status;
}

/** Pause, resume, marker, stop and discard, shared by this screen and the bar. */
export function useRecordingControls(fromBar: boolean, onError: (message: string) => void) {
  const [busy, setBusy] = useState(false);
  const run = async (f: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    try {
      await f();
    } catch (e) {
      onError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return {
    busy,
    pause: (paused: boolean) => run(() => (paused ? api.resumeRecording() : api.pauseRecording())),
    marker: () => run(() => api.addMarker()),
    stop: () => run(() => api.stopRecording(fromBar)),
    discard: () => run(() => api.cancelRecording()),
  };
}

export function Recording() {
  const status = useRecordingStatus();
  const { busy, pause, marker, stop, discard } = useRecordingControls(false, useStore((s) => s.setError));
  const keys = { ...DEFAULT_HOTKEYS, ...useStore((s) => s.settings.hotkeys) };
  const counting = status !== null && status.elapsedMs < status.countdownMs;
  const paused = status?.paused ?? false;

  return (
    <div className="recording">
      <div className={`rec-indicator${paused ? " paused" : ""}`}>
        <span className={`dot${paused ? "" : " pulse"}`} />
        {paused ? "Paused" : counting ? "Starting" : "Recording"}
      </div>
      <div className="rec-timer" aria-live="polite">
        {counting ? Math.ceil((status.countdownMs - status.elapsedMs) / 1000) : formatTime(Math.max(0, (status?.elapsedMs ?? 0) - (status?.countdownMs ?? 0)))}
      </div>
      <div className="rec-actions">
        <button className="btn primary big" disabled={busy} onClick={stop}>
          Stop
        </button>
        <button className="btn" disabled={busy || !status} onClick={() => pause(paused)}>
          {paused ? "Resume" : "Pause"}
        </button>
        <button className="btn" disabled={busy || paused} onClick={marker}>
          Marker
        </button>
        <button className="btn ghost" disabled={busy} onClick={discard}>
          Discard
        </button>
      </div>
      <p className="note">
        <kbd>{hotkeyLabel(keys.toggle)}</kbd> stops, <kbd>{hotkeyLabel(keys.pause)}</kbd> pauses, <kbd>{hotkeyLabel(keys.marker)}</kbd> marks a moment. Nothing is recorded
        while paused.
      </p>
    </div>
  );
}
