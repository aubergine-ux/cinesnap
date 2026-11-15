import { PRESETS, outputFps, outputSize, resolvePreset } from "../../lib/layout";
import { mainCrop } from "../../lib/edit";
import { useStore } from "../../lib/store";
import { formatTime } from "../../lib/timeline";
import type { Project, Quality } from "../../lib/types";
import { Section, Segmented, TextInput } from "../controls";

const DESTINATIONS = ["readme", "readme-mp4", "x", "youtube", "shorts"];
const SIZES = ["720p", "1080p", "1440p", "4k", "small"];

export function ExportPanel({ onExport }: { onExport: () => void }) {
  const project = useStore((s) => s.project)!;
  const seconds = useStore((s) => (s.motion?.durationMs ?? 0) / 1000);
  const settings = useStore((s) => s.settings);
  const { update, saveSettings } = useStore.getState();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });

  const exp = project.export;
  const preset = resolvePreset(exp);
  const size = outputSize(project.frame.aspect, mainCrop(project), exp);
  const pick = (id: string) => {
    const p = PRESETS.find((x) => x.id === id)!;
    set("export:preset", (proj) => {
      proj.export.preset = p.id;
      proj.export.format = p.format;
    });
  };
  const tooLong = preset.maxSeconds !== null && seconds > preset.maxSeconds;

  return (
    <>
      <Section title="Where is it going?">
        <div className="preset-grid">
          {DESTINATIONS.map((id) => {
            const p = PRESETS.find((x) => x.id === id)!;
            return (
              <button key={id} className={`preset ${preset.id === id ? "active" : ""}`} onClick={() => pick(id)}>
                <strong>{p.label}</strong>
                <span>{p.hint}</span>
              </button>
            );
          })}
        </div>
        <div className="stack">
          <span className="row-label">Or just a size</span>
          <Segmented<string> value={SIZES.includes(preset.id) ? preset.id : ""} onChange={pick} options={SIZES.map((id) => ({ value: id, label: id === "small" ? "GIF 480" : PRESETS.find((p) => p.id === id)!.label }))} />
        </div>
        {preset.fps === null && (
          <div className="stack">
            <span className="row-label">Frame rate</span>
            <Segmented<number>
              value={exp.fps}
              onChange={(v) => set("export:fps", (p) => void (p.export.fps = v))}
              options={[
                { value: 30, label: "30 fps" },
                { value: 60, label: "60 fps" },
              ]}
            />
          </div>
        )}
        <div className="stack">
          <span className="row-label">Quality</span>
          <Segmented<Quality>
            value={exp.quality}
            onChange={(v) => set("export:quality", (p) => void (p.export.quality = v))}
            options={[
              { value: "low", label: "Small file" },
              { value: "medium", label: "Balanced" },
              { value: "high", label: "Best" },
            ]}
          />
        </div>
        <p className="hint">
          {exp.format.toUpperCase()} · {size.width}×{size.height} · {outputFps(exp)} fps · {formatTime(seconds * 1000, false)}
          {preset.aspect ? ` · ${preset.aspect} for this preset` : ""}
          {preset.maxBytes && preset.maxBytes < 64 * 1024 * 1024 ? ` · kept under ${Math.round(preset.maxBytes / 1024 / 1024)} MB` : ""}
        </p>
        {tooLong && (
          <p className="hint warn">
            This video is {formatTime(seconds * 1000, false)} long; {preset.label} accepts at most {formatTime(preset.maxSeconds! * 1000, false)}.
          </p>
        )}
        <button className="btn primary wide" onClick={onExport}>
          Export {exp.format.toUpperCase()}
        </button>
      </Section>

      <Section title="Share server" defaultOpen={Boolean(settings.shareServer)}>
        <TextInput label="Address" type="url" placeholder="https://share.example.com" value={settings.shareServer ?? ""} onChange={(v) => void saveSettings({ shareServer: v })} />
        <TextInput label="Upload token" type="password" value={settings.shareToken ?? ""} onChange={(v) => void saveSettings({ shareToken: v })} />
        <p className="hint">Optional. With your own share server set up, a finished export can be uploaded for a link with a player page and an embed code. The token is stored on this computer only.</p>
      </Section>
    </>
  );
}
