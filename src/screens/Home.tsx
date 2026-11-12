import { useEffect, useMemo, useState } from "react";
import { Segmented, Select, Toggle } from "../components/controls";
import { api, errorMessage } from "../lib/api";
import { useStore } from "../lib/store";
import { formatTime } from "../lib/timeline";
import type { Rect, RecordingOptions } from "../lib/types";

const NONE = "__none__";

export function Home() {
  const { capabilities, sources, recent, refreshHome, openProject, setScreen, setError } = useStore();
  const [target, setTarget] = useState<string>("");
  const [mic, setMic] = useState(NONE);
  const [webcam, setWebcam] = useState(NONE);
  const [fps, setFps] = useState(60);
  const [shortcuts, setShortcuts] = useState(true);
  const [useRegion, setUseRegion] = useState(false);
  const [region, setRegion] = useState<Rect>({ x: 0, y: 0, width: 1280, height: 720 });
  const [starting, setStarting] = useState(false);
  const [download, setDownload] = useState<number | null>(null);

  const targets = useMemo(() => {
    if (!sources) return [];
    if (sources.portal) return [{ value: "portal", label: "Screen or window (chosen in system dialog)" }];
    return [
      ...sources.screens.map((s) => ({ value: `screen:${s.id}`, label: `${s.name} — ${s.width}×${s.height}` })),
      ...sources.windows.map((w) => ({ value: `window:${w.id}`, label: `Window: ${w.title}` })),
    ];
  }, [sources]);

  useEffect(() => {
    if (targets.length && !targets.some((t) => t.value === target)) setTarget(targets[0].value);
  }, [targets, target]);

  useEffect(() => {
    // Default region: 1280x720 centred on the chosen screen.
    const s = sources?.screens.find((x) => `screen:${x.id}` === target);
    if (s) setRegion({ x: Math.max(0, Math.round((s.width - 1280) / 2)), y: Math.max(0, Math.round((s.height - 720) / 2)), width: Math.min(1280, s.width), height: Math.min(720, s.height) });
  }, [target, sources]);

  const regionAllowed = target === "portal" || target.startsWith("screen:");
  const ffmpegMissing = capabilities !== null && capabilities.ffmpeg === null;

  const record = async () => {
    if (!target || starting) return;
    const [kind, id] = target === "portal" ? ["portal", undefined] : (target.split(/:(.*)/s) as [string, string]);
    const options: RecordingOptions = {
      target: { kind: kind as RecordingOptions["target"]["kind"], ...(id !== undefined ? { id } : {}) },
      region: useRegion && regionAllowed ? region : null,
      mic: mic === NONE ? null : mic,
      webcam: webcam === NONE ? null : webcam,
      fps,
      shortcuts,
    };
    setStarting(true);
    try {
      await api.startRecording(options);
      setScreen("recording");
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setStarting(false);
    }
  };

  const downloadFfmpeg = async () => {
    setDownload(0);
    const off = api.onFfmpegProgress((p) => setDownload(p.fraction));
    try {
      await api.downloadFfmpeg();
      await refreshHome();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      off();
      setDownload(null);
    }
  };

  const remove = async (path: string) => {
    try {
      await api.deleteProject(path);
      await refreshHome();
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const notes = capabilities?.notes ?? [];

  return (
    <div className="home">
      <div className="home-card">
        <div className="brand">
          <span className="logo" aria-hidden />
          <div>
            <h1>Cinesnap</h1>
            <p>Record your screen. Get a polished demo.</p>
          </div>
        </div>

        {ffmpegMissing && (
          <div className="banner warn">
            <div>
              <strong>ffmpeg is required</strong>
              <span>Cinesnap uses ffmpeg to record and export. It was not found on this system.</span>
            </div>
            <button className="btn" disabled={download !== null} onClick={downloadFfmpeg}>
              {download === null ? "Download" : `${Math.round(download * 100)}%`}
            </button>
          </div>
        )}

        <div className="field">
          <span className="field-label">Capture</span>
          <Select value={target} options={targets.length ? targets : [{ value: "", label: "No sources found" }]} onChange={setTarget} />
        </div>

        {regionAllowed && (
          <div className="field">
            <Toggle label="Record a region" value={useRegion} onChange={setUseRegion} />
            {useRegion && (
              <div className="region">
                {(["x", "y", "width", "height"] as const).map((k) => (
                  <label key={k}>
                    <span>{k === "width" ? "W" : k === "height" ? "H" : k.toUpperCase()}</span>
                    <input
                      type="number"
                      min={k === "width" || k === "height" ? 16 : 0}
                      value={region[k]}
                      onChange={(e) => setRegion({ ...region, [k]: Math.max(0, Math.round(Number(e.target.value) || 0)) })}
                    />
                  </label>
                ))}
              </div>
            )}
          </div>
        )}

        <div className="field-grid">
          <div className="field">
            <span className="field-label">Microphone</span>
            <Select value={mic} options={[{ value: NONE, label: "Off" }, ...(sources?.mics ?? []).map((m) => ({ value: m.id, label: m.name }))]} onChange={setMic} />
          </div>
          <div className="field">
            <span className="field-label">Webcam</span>
            <Select value={webcam} options={[{ value: NONE, label: "Off" }, ...(sources?.webcams ?? []).map((m) => ({ value: m.id, label: m.name }))]} onChange={setWebcam} />
          </div>
        </div>

        <div className="field inline">
          <span className="field-label">Frame rate</span>
          <Segmented
            value={fps}
            onChange={setFps}
            options={[
              { value: 30, label: "30 fps" },
              { value: 60, label: "60 fps" },
            ]}
          />
        </div>

        <div className="field inline">
          <span className="field-label" title="Shortcuts such as Ctrl+K and keys like Enter can be shown in the video. Text you type is never recorded.">
            Record shortcuts
          </span>
          <Toggle label="" value={shortcuts} onChange={setShortcuts} />
        </div>

        <button className="record" disabled={!target || starting || ffmpegMissing} onClick={record}>
          <span className="dot" />
          {starting ? "Starting…" : "Start recording"}
        </button>

        {notes.map((n) => (
          <p className="note" key={n}>
            {n}
          </p>
        ))}
      </div>

      <div className="recent">
        <h2>Recent recordings</h2>
        {recent.length === 0 && <p className="empty">Your recordings will show up here.</p>}
        <ul>
          {recent.map((r) => (
            <li key={r.path}>
              <button className="recent-item" onClick={() => openProject(r.path)}>
                <span className="thumb" aria-hidden>
                  ▶
                </span>
                <span className="recent-text">
                  <strong>{r.name}</strong>
                  <small>
                    {new Date(r.createdAt).toLocaleString()} · {formatTime(r.durationMs, false)}
                  </small>
                </span>
              </button>
              <button className="icon-btn ghost" title="Delete recording" onClick={() => remove(r.path)}>
                ✕
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
