import { useEffect, useMemo, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { useStore } from "../lib/store";
import { formatTime } from "../lib/timeline";
import type { Suggestion } from "../lib/types";
import { Slider, Toggle } from "./controls";

/** Find filler words and long silences, and let the user review them before anything is cut. */
export function TidyModal({ onClose }: { onClose: () => void }) {
  const hasTranscript = useStore((s) => Boolean(s.project?.transcript?.words.length));
  const hasMic = useStore((s) => Boolean(s.project?.clips.some((c) => c.kind === "recording" && c.source.mic)));
  const [fillers, setFillers] = useState(hasTranscript);
  const [silences, setSilences] = useState(true);
  const [minMs, setMinMs] = useState(1000);
  const [found, setFound] = useState<Suggestion[] | null>(null);
  const [skip, setSkip] = useState<Set<number>>(new Set());
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const seq = useRef(0);

  useEffect(() => {
    const { project, projectPath } = useStore.getState();
    if (!project || !projectPath) return;
    const mine = ++seq.current;
    setBusy(true);
    setError("");
    const timer = setTimeout(() => {
      api
        .suggestCuts(projectPath, project, fillers && hasTranscript, silences && hasMic, minMs)
        .then((list) => {
          if (mine !== seq.current) return;
          setFound(list);
          setSkip(new Set());
        })
        .catch((e) => {
          if (mine === seq.current && errorMessage(e) !== "cancelled") setError(errorMessage(e));
        })
        .finally(() => mine === seq.current && setBusy(false));
    }, 250);
    return () => clearTimeout(timer);
  }, [fillers, silences, minMs, hasTranscript, hasMic]);

  const chosen = useMemo(() => (found ?? []).filter((_, i) => !skip.has(i)), [found, skip]);
  const saved = chosen.reduce((s, c) => s + c.endMs - c.startMs, 0);

  const preview = (s: Suggestion) => {
    // Play from a second before to hear what the cut would join.
    const st = useStore.getState();
    st.seekSource(Math.max(0, s.startMs - 1000));
    st.setPlaying(true);
  };

  const apply = () => {
    const st = useStore.getState();
    st.setPlaying(false);
    st.applyCuts(chosen.map((c) => ({ startMs: c.startMs, endMs: c.endMs })));
    onClose();
  };

  const close = () => {
    seq.current++;
    void api.cancelJob("silence").catch(() => undefined);
    useStore.getState().setPlaying(false);
    onClose();
  };

  return (
    <div className="modal-backdrop">
      <div className="modal wide" role="dialog" aria-modal="true" aria-label="Remove fillers and silences">
        <h2>Remove fillers and silences</h2>
        <Toggle label={hasTranscript ? 'Filler words ("um", "uh")' : "Filler words (generate captions first)"} value={fillers && hasTranscript} onChange={(v) => hasTranscript && setFillers(v)} />
        <Toggle label={hasMic ? "Long silences" : "Long silences (no microphone audio)"} value={silences && hasMic} onChange={(v) => hasMic && setSilences(v)} />
        {silences && hasMic && <Slider label="Silences longer than" value={minMs} min={500} max={5000} step={100} format={(v) => `${(v / 1000).toFixed(1)}s`} onChange={setMinMs} />}

        <div className="suggestions">
          {busy && <p className="hint">Listening…</p>}
          {!busy && error && <pre className="error-text">{error}</pre>}
          {!busy && !error && found?.length === 0 && <p className="hint">Nothing to remove: no fillers or long silences were found.</p>}
          {!busy &&
            found?.map((s, i) => (
              <label key={`${s.kind}${s.startMs}`} className={`suggestion ${skip.has(i) ? "off" : ""}`}>
                <input
                  type="checkbox"
                  checked={!skip.has(i)}
                  onChange={() =>
                    setSkip((old) => {
                      const next = new Set(old);
                      if (!next.delete(i)) next.add(i);
                      return next;
                    })
                  }
                />
                <span className={`tag ${s.kind}`}>{s.kind === "filler" ? "filler" : "silence"}</span>
                <span className="suggestion-label">{s.kind === "filler" ? `"${s.label}"` : s.label}</span>
                <span className="suggestion-time">{formatTime(s.startMs)}</span>
                <button
                  className="icon-btn"
                  title="Play this spot"
                  onClick={(e) => {
                    e.preventDefault();
                    preview(s);
                  }}
                >
                  ▶
                </button>
              </label>
            ))}
        </div>

        <p className="hint">
          {chosen.length ? `${chosen.length} cuts, ${(saved / 1000).toFixed(1)} s shorter. ` : ""}
          Nothing changes until you apply, and one undo brings everything back.
        </p>
        <div className="modal-actions">
          <button className="btn" onClick={close}>
            Cancel
          </button>
          <button className="btn primary" disabled={busy || !chosen.length} onClick={apply}>
            Apply {chosen.length || ""} cuts
          </button>
        </div>
      </div>
    </div>
  );
}
