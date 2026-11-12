import { pickAudio } from "../../lib/api";
import { useStore } from "../../lib/store";
import type { Music, Project } from "../../lib/types";
import { JobBar, Section, Slider, Toggle } from "../controls";

const pct = (v: number) => `${Math.round(v * 100)}%`;
const db = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;
const ms = (v: number) => `${(v / 1000).toFixed(1)}s`;

const DEFAULT_MUSIC: Music = { source: "", volume: 0.35, duck: true, duckDb: -14, fadeInMs: 800, fadeOutMs: 2500 };

export function AudioPanel() {
  const project = useStore((s) => s.project)!;
  const tracks = useStore((s) => s.musicTracks);
  const job = useStore((s) => s.jobs.audio);
  const stale = useStore((s) => s.audioStale);
  const { update, cancelJob } = useStore.getState();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });

  const audio = project.audio;
  const hasMic = project.clips.some((c) => c.kind === "recording" && c.source.mic);
  const music = audio.music;
  const setMusic = (patch: Partial<Music>, key = "music") => set(key, (p) => void (p.audio.music = { ...(p.audio.music ?? DEFAULT_MUSIC), ...patch }));
  const enhanced = audio.normalize && audio.denoise > 0 && audio.eq.enabled;
  const isFile = Boolean(music?.source) && !music!.source.startsWith("builtin:");

  return (
    <>
      {stale && <JobBar label="Preparing the soundtrack…" fraction={job?.fraction ?? 0} onCancel={() => cancelJob("audio")} />}

      <Section title="Voice">
        {!hasMic && <p className="hint">This project was recorded without a microphone, so there is no voice to clean up.</p>}
        {hasMic && (
          <>
            <button
              className={`btn wide ${enhanced ? "active" : ""}`}
              onClick={() =>
                set("audio:enhance", (p) => {
                  const on = !enhanced;
                  p.audio.normalize = on;
                  p.audio.denoise = on ? 0.5 : 0;
                  p.audio.eq.enabled = on;
                })
              }
            >
              {enhanced ? "✓ Voice enhanced" : "Enhance voice"}
            </button>
            <p className="hint">One click: reduces background noise, evens out the loudness and adds a little clarity. Fine-tune below.</p>
            <Slider label="Volume" value={audio.micVolume} min={0} max={2} step={0.01} format={pct} onChange={(v) => set("audio:vol", (p) => void (p.audio.micVolume = v))} />
            <Slider label="Noise reduction" value={audio.denoise} min={0} max={1} step={0.05} format={(v) => (v === 0 ? "Off" : pct(v))} onChange={(v) => set("audio:denoise", (p) => void (p.audio.denoise = v))} />
            <Toggle label="Even out loudness" value={audio.normalize} onChange={(v) => set("audio:norm", (p) => void (p.audio.normalize = v))} />
            <Toggle label="Voice EQ" value={audio.eq.enabled} onChange={(v) => set("audio:eq", (p) => void (p.audio.eq.enabled = v))} />
            {audio.eq.enabled && (
              <>
                <Slider label="Cut rumble below" value={audio.eq.lowCutHz} min={40} max={200} step={5} format={(v) => `${v} Hz`} onChange={(v) => set("eq:low", (p) => void (p.audio.eq.lowCutHz = v))} />
                <Slider label="Warmth" value={audio.eq.bassDb} min={-8} max={8} step={0.5} format={db} onChange={(v) => set("eq:bass", (p) => void (p.audio.eq.bassDb = v))} />
                <Slider label="Clarity" value={audio.eq.presenceDb} min={-8} max={8} step={0.5} format={db} onChange={(v) => set("eq:pres", (p) => void (p.audio.eq.presenceDb = v))} />
                <Slider label="Air" value={audio.eq.airDb} min={-8} max={8} step={0.5} format={db} onChange={(v) => set("eq:air", (p) => void (p.audio.eq.airDb = v))} />
              </>
            )}
          </>
        )}
      </Section>

      <Section title="Music">
        <div className="track-list">
          <button className={`track ${!music?.source ? "active" : ""}`} onClick={() => set("music:none", (p) => void (p.audio.music = undefined))}>
            <strong>No music</strong>
          </button>
          {tracks.map((t) => (
            <button key={t.id} className={`track ${music?.source === `builtin:${t.id}` ? "active" : ""}`} onClick={() => setMusic({ source: `builtin:${t.id}` }, "music:pick")}>
              <strong>{t.name}</strong>
              <span>
                {t.mood} · {t.bpm} bpm
              </span>
            </button>
          ))}
          <button
            className={`track ${isFile ? "active" : ""}`}
            onClick={async () => {
              const path = await pickAudio();
              if (path) setMusic({ source: path }, "music:pick");
            }}
          >
            <strong>{isFile ? music!.source.split(/[\\/]/).pop() : "Your own file…"}</strong>
            <span>MP3, WAV, M4A, OGG, FLAC</span>
          </button>
        </div>
        {music?.source && (
          <>
            <Slider label="Volume" value={music.volume} min={0} max={1} step={0.01} format={pct} onChange={(v) => setMusic({ volume: v }, "music:vol")} />
            <Toggle label="Lower under speech" value={music.duck} onChange={(v) => setMusic({ duck: v })} />
            {music.duck && <Slider label="Lower by" value={music.duckDb} min={-30} max={-3} step={1} format={db} onChange={(v) => setMusic({ duckDb: v }, "music:duck")} />}
            <Slider label="Fade in" value={music.fadeInMs} min={0} max={5000} step={100} format={ms} onChange={(v) => setMusic({ fadeInMs: v }, "music:in")} />
            <Slider label="Fade out" value={music.fadeOutMs} min={0} max={8000} step={100} format={ms} onChange={(v) => setMusic({ fadeOutMs: v }, "music:out")} />
          </>
        )}
        <p className="hint">The built-in tracks are generated by Cinesnap itself, so they are free to use anywhere. Music loops for as long as the video runs.</p>
      </Section>
    </>
  );
}
