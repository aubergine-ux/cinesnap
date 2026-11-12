import { useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { useStore } from "../lib/store";
import { formatTime } from "../lib/timeline";

export function Recording() {
  const { openProject, setScreen, setError, refreshHome } = useStore();
  const [elapsed, setElapsed] = useState(0);
  const [busy, setBusy] = useState(false);

  useEffect(() => api.onRecordingTick((p) => setElapsed(p.elapsedMs)), []);

  const stop = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const path = await api.stopRecording();
      await openProject(path);
      if (useStore.getState().screen !== "editor") setScreen("home");
    } catch (e) {
      setError(errorMessage(e));
      setScreen("home");
    }
  };

  const cancel = async () => {
    if (busy) return;
    setBusy(true);
    try {
      await api.cancelRecording();
    } catch (e) {
      setError(errorMessage(e));
    }
    setScreen("home");
    void refreshHome();
  };

  return (
    <div className="recording">
      <div className="rec-indicator">
        <span className="dot pulse" />
        Recording
      </div>
      <div className="rec-timer">{formatTime(elapsed)}</div>
      <div className="rec-actions">
        <button className="btn primary big" disabled={busy} onClick={stop}>
          {busy ? "Finishing…" : "Stop"}
        </button>
        <button className="btn ghost" disabled={busy} onClick={cancel}>
          Discard
        </button>
      </div>
      <p className="note">Clicks and cursor movement are being logged so zooms can be added automatically.</p>
    </div>
  );
}
