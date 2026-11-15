import { useStore, type PanelTab } from "../lib/store";
import { AudioPanel } from "./panels/AudioPanel";
import { CaptionsPanel } from "./panels/CaptionsPanel";
import { ExportPanel } from "./panels/ExportPanel";
import { Inspector } from "./panels/Inspector";
import { StylePanel } from "./panels/StylePanel";
import { VideoPanel } from "./panels/VideoPanel";

const TABS: { id: PanelTab; label: string }[] = [
  { id: "style", label: "Style" },
  { id: "video", label: "Video" },
  { id: "audio", label: "Audio" },
  { id: "captions", label: "Captions" },
  { id: "export", label: "Export" },
];

export function SettingsPanel({ onExport, onTidy }: { onExport: () => void; onTidy: () => void }) {
  const tab = useStore((s) => s.panelTab);
  const { setTab } = useStore.getState();
  return (
    <aside className="panel">
      <nav className="tabs" role="tablist">
        {TABS.map((t) => (
          <button key={t.id} role="tab" aria-selected={tab === t.id} className={tab === t.id ? "active" : ""} onClick={() => setTab(t.id)}>
            {t.label}
          </button>
        ))}
      </nav>
      <div className="panel-body">
        <Inspector />
        {tab === "style" && <StylePanel />}
        {tab === "video" && <VideoPanel />}
        {tab === "audio" && <AudioPanel />}
        {tab === "captions" && <CaptionsPanel onTidy={onTidy} />}
        {tab === "export" && <ExportPanel onExport={onExport} />}
      </div>
    </aside>
  );
}
