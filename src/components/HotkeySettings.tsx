import { useState } from "react";
import { api, errorMessage } from "../lib/api";
import { DEFAULT_HOTKEYS, accelFromEvent, hotkeyLabel } from "../lib/hotkeys";
import { useStore } from "../lib/store";
import type { Hotkeys } from "../lib/types";
import { Toggle } from "./controls";

const NAMES: { key: keyof Hotkeys; label: string }[] = [
  { key: "toggle", label: "Start / stop" },
  { key: "pause", label: "Pause / resume" },
  { key: "marker", label: "Marker" },
];

function KeyField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const [listening, setListening] = useState(false);
  return (
    <label className="key-field">
      <span>{label}</span>
      <button
        type="button"
        className={`btn small ${listening ? "active" : ""}`}
        aria-label={`${label}: ${hotkeyLabel(value)}. Press to change.`}
        onClick={() => setListening(true)}
        onBlur={() => setListening(false)}
        onKeyDown={(e) => {
          if (!listening) return;
          e.preventDefault();
          if (e.key === "Escape") return setListening(false);
          const accel = accelFromEvent(e);
          if (accel) {
            onChange(accel);
            setListening(false);
          }
        }}
      >
        {listening ? "Press keys…" : <kbd>{hotkeyLabel(value)}</kbd>}
      </button>
    </label>
  );
}

export function HotkeySettings() {
  const { settings, saveSettings, capabilities } = useStore();
  const [notes, setNotes] = useState<string[]>([]);
  const keys = { ...DEFAULT_HOTKEYS, ...settings.hotkeys };
  const wayland = capabilities?.platform === "linux-wayland";
  const linux = capabilities?.platform.startsWith("linux");

  const change = async (k: keyof Hotkeys, v: string) => {
    await saveSettings({ hotkeys: { ...keys, [k]: v } });
    try {
      setNotes(await api.applyHotkeys());
    } catch (e) {
      setNotes([errorMessage(e)]);
    }
  };

  return (
    <details className="hotkeys">
      <summary>Hotkeys and recording bar</summary>
      {wayland ? (
        <p className="hint">
          On Wayland your desktop owns global shortcuts: the first recording asks it to add these, and they can be changed in its keyboard settings.
        </p>
      ) : (
        NAMES.map((n) => <KeyField key={n.key} label={n.label} value={keys[n.key]} onChange={(v) => void change(n.key, v)} />)
      )}
      {notes.map((n) => (
        <p className="note" key={n}>
          {n}
        </p>
      ))}
      {linux && (
        <>
          <Toggle label="Keep the recording bar visible" value={settings.keepBarVisible ?? false} onChange={(v) => void saveSettings({ keepBarVisible: v })} />
          <p className="hint">
            Linux cannot leave a window out of a screen recording. With one screen, the bar is shown for the countdown and then hidden; the hotkeys and this window still work.
          </p>
        </>
      )}
    </details>
  );
}
