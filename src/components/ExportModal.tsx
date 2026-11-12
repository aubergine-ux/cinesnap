import { useEffect, useRef, useState } from "react";
import { api, copyText, errorMessage } from "../lib/api";
import { useStore } from "../lib/store";
import type { ExportProgress, Shared } from "../lib/types";
import { JobBar } from "./controls";

type Phase = { kind: "running" } | { kind: "done" } | { kind: "error"; message: string };
type Share = { kind: "idle" } | { kind: "uploading" } | { kind: "done"; shared: Shared } | { kind: "error"; message: string };

const STAGES: Record<string, string> = { audio: "Mixing the soundtrack", render: "Rendering", palette: "Choosing colours", encode: "Encoding", starting: "Starting" };

export function ExportModal({ outPath, title, onClose }: { outPath: string; title: string; onClose: () => void }) {
  const [progress, setProgress] = useState<ExportProgress>({ frame: 0, total: 0, stage: "starting" });
  const [phase, setPhase] = useState<Phase>({ kind: "running" });
  const [share, setShare] = useState<Share>({ kind: "idle" });
  const [copied, setCopied] = useState("");
  const upload = useStore((s) => s.jobs.share);
  const canShare = useStore((s) => Boolean(s.settings.shareServer && s.settings.shareToken));
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
  const stage = STAGES[progress.stage] ?? progress.stage;

  const cancel = () => {
    cancelled.current = true;
    api.cancelExport().catch(() => undefined);
  };

  const startShare = () => {
    setShare({ kind: "uploading" });
    api
      .shareVideo(outPath, title)
      .then((shared) => setShare({ kind: "done", shared }))
      .catch((e) => setShare(errorMessage(e) === "cancelled" ? { kind: "idle" } : { kind: "error", message: errorMessage(e) }))
      .finally(() => useStore.setState((s) => ({ jobs: Object.fromEntries(Object.entries(s.jobs).filter(([k]) => k !== "share")) })));
  };

  const copy = async (what: string, text: string) => {
    setCopied((await copyText(text)) ? what : "failed");
    setTimeout(() => setCopied(""), 1800);
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
              {phase.kind === "done" ? "Saved." : progress.stage === "render" && progress.total > 0 ? `${stage} · frame ${progress.frame} of ${progress.total} · ${Math.round(fraction * 100)}%` : `${stage}…`}
            </p>
          </>
        )}
        {phase.kind === "error" && <pre className="error-text">{phase.message}</pre>}

        {phase.kind === "done" && canShare && (
          <div className="share">
            {share.kind === "idle" && (
              <button className="btn wide" onClick={startShare}>
                Upload and get a link
              </button>
            )}
            {share.kind === "uploading" && <JobBar label="Uploading…" fraction={upload?.fraction ?? 0} onCancel={() => void api.cancelJob("share")} />}
            {share.kind === "error" && (
              <>
                <pre className="error-text">{share.message}</pre>
                <button className="btn wide" onClick={startShare}>
                  Try again
                </button>
              </>
            )}
            {share.kind === "done" && (
              <>
                <p className="path share-link" title={share.shared.url}>
                  {share.shared.url}
                </p>
                <div className="btn-row">
                  <button className="btn small" onClick={() => void copy("link", share.shared.url)}>
                    {copied === "link" ? "Copied" : "Copy link"}
                  </button>
                  <button className="btn small" onClick={() => void copy("embed", share.shared.embedHtml)}>
                    {copied === "embed" ? "Copied" : "Copy embed code"}
                  </button>
                </div>
                {copied === "failed" && <p className="hint warn">Could not reach the clipboard; select the link above instead.</p>}
              </>
            )}
          </div>
        )}
        {phase.kind === "done" && !canShare && <p className="hint">To share a link instead of a file, add a share server in the Export tab.</p>}

        <div className="modal-actions">
          {phase.kind === "running" ? (
            <button className="btn" onClick={cancel}>
              Cancel
            </button>
          ) : (
            <button className="btn primary" disabled={share.kind === "uploading"} onClick={onClose}>
              Close
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
