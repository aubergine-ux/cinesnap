import { useEffect, useRef, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { useStore } from "../lib/store";
import type { ExportProgress } from "../lib/types";

type Phase = { kind: "running" } | { kind: "done" } | { kind: "error"; message: string };

export function ExportModal({ outPath, onClose }: { outPath: string; onClose: () => void }) {
  const [progress, setProgress] = useState<ExportProgress>({ frame: 0, total: 0, stage: "starting" });
  const [phase, setPhase] = useState<Phase>({ kind: "running" });
  const cancelled = useRef(false);
  const started = useRef(false);

  useEffect(() => {
    const off = api.onExportProgress(setProgress);
    if (!started.current) {
      started.current = true;
      const { project, projectPath } = useStore.getState();
      if (project && projectPath) {
        api
          .exportVideo(projectPath, project, outPath)
          .then(() => setPhase({ kind: "done" }))
          .catch((e) => (cancelled.current ? onClose() : setPhase({ kind: "error", message: errorMessage(e) })));
      }
    }
    return off;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const fraction = phase.kind === "done" ? 1 : progress.total > 0 ? Math.min(1, progress.frame / progress.total) : 0;

  const cancel = () => {
    cancelled.current = true;
    api.cancelExport().catch(() => undefined);
  };

  return (
    <div className="modal-backdrop">
      <div className="modal" role="dialog" aria-modal="true" aria-label="Export">
        <h2>{phase.kind === "done" ? "Export complete" : phase.kind === "error" ? "Export failed" : "Exporting…"}</h2>
        <p className="path" title={outPath}>
          {outPath}
        </p>
        {phase.kind !== "error" && (
          <>
            <div className="progress">
              <span style={{ width: `${fraction * 100}%` }} />
            </div>
            <p className="hint">
              {phase.kind === "done" ? "Saved." : progress.total > 0 ? `${progress.stage} · frame ${progress.frame} of ${progress.total} · ${Math.round(fraction * 100)}%` : progress.stage}
            </p>
          </>
        )}
        {phase.kind === "error" && <pre className="error-text">{phase.message}</pre>}
        <div className="modal-actions">
          {phase.kind === "running" ? (
            <button className="btn" onClick={cancel}>
              Cancel
            </button>
          ) : (
            <button className="btn primary" onClick={onClose}>
              Close
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
