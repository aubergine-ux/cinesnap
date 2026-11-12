import { useEffect, useState } from "react";
import { ExportModal } from "../components/ExportModal";
import { Preview } from "../components/Preview";
import { SettingsPanel } from "../components/SettingsPanel";
import { Timeline } from "../components/Timeline";
import { errorMessage, pickSavePath } from "../lib/api";
import { useStore } from "../lib/store";

function projectName(path: string): string {
  const base = path.split(/[\\/]/).filter(Boolean).pop() ?? "recording";
  return base.replace(/\.cinesnap$/i, "");
}

export function Editor() {
  const path = useStore((s) => s.projectPath)!;
  const format = useStore((s) => s.project!.export.format);
  const panelOpen = useStore((s) => s.panelOpen);
  const { closeProject, togglePanel, setError } = useStore.getState();
  const [exportPath, setExportPath] = useState<string | null>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = t && (t.tagName === "TEXTAREA" || t.tagName === "SELECT" || (t.tagName === "INPUT" && (t as HTMLInputElement).type !== "range"));
      if (typing || exportPath) return;
      const st = useStore.getState();
      const mod = e.ctrlKey || e.metaKey;
      const key = e.key.toLowerCase();
      if (mod && key === "z") {
        e.preventDefault();
        if (e.shiftKey) st.redo();
        else st.undo();
      } else if (mod && key === "y") {
        e.preventDefault();
        st.redo();
      } else if (mod) {
        return;
      } else if (e.key === " ") {
        e.preventDefault();
        (document.activeElement as HTMLElement | null)?.blur?.();
        st.setPlaying(!st.playing);
      } else if (e.key === "Delete" || e.key === "Backspace") {
        if (st.selection) {
          e.preventDefault();
          st.deleteSelection();
        }
      } else if (key === "s") {
        e.preventDefault();
        st.markOrCut();
      } else if (key === "c") {
        st.setCutMode(!st.cutMode);
      } else if (e.key === "Escape") {
        useStore.setState({ selection: null, cutMode: false, cutMark: null });
      } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
        e.preventDefault();
        st.setPlaying(false);
        st.setFrame(st.frame + (e.key === "ArrowLeft" ? -1 : 1) * (e.shiftKey ? 10 : 1));
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [exportPath]);

  const startExport = async () => {
    try {
      useStore.getState().setPlaying(false);
      const out = await pickSavePath(projectName(path), format);
      if (out) setExportPath(out);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <div className={`editor ${panelOpen ? "" : "panel-closed"}`}>
      <header className="topbar">
        <button className="btn ghost small" onClick={closeProject}>
          ‹ Recordings
        </button>
        <span className="title">{projectName(path)}</span>
        <span className="grow" />
        <button className="btn primary small" onClick={startExport}>
          Export
        </button>
        <button className="icon-btn" title={panelOpen ? "Hide settings" : "Show settings"} onClick={togglePanel}>
          {panelOpen ? "⇥" : "⇤"}
        </button>
      </header>
      <main className="workspace">
        <Preview />
        <Timeline />
      </main>
      {panelOpen && <SettingsPanel onExport={startExport} />}
      {exportPath && <ExportModal outPath={exportPath} onClose={() => setExportPath(null)} />}
    </div>
  );
}
