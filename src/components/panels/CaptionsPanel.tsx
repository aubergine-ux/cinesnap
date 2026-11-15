import { useMemo, useState } from "react";
import { errorMessage, api, pickSaveAs } from "../../lib/api";
import { isWordCut } from "../../lib/edit";
import { useStore } from "../../lib/store";
import type { CaptionSettings, Project, Word } from "../../lib/types";
import { AlphaColorInput, ColorInput, JobBar, Section, Segmented, SelectRow, Slider, Toggle } from "../controls";
import { FONT_OPTIONS } from "./Inspector";

const LANGUAGES = [
  { value: "", label: "Detect" },
  { value: "en", label: "English" },
  { value: "es", label: "Spanish" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "it", label: "Italian" },
  { value: "pt", label: "Portuguese" },
  { value: "nl", label: "Dutch" },
  { value: "pl", label: "Polish" },
  { value: "tr", label: "Turkish" },
  { value: "ru", label: "Russian" },
  { value: "uk", label: "Ukrainian" },
  { value: "hi", label: "Hindi" },
  { value: "ja", label: "Japanese" },
  { value: "ko", label: "Korean" },
  { value: "zh", label: "Chinese" },
];

function Generate({ hasTranscript }: { hasTranscript: boolean }) {
  const speech = useStore((s) => s.speech);
  const saved = useStore((s) => s.settings.speechModel);
  const job = useStore((s) => s.jobs.transcribe);
  const hasMic = useStore((s) => s.project!.clips.some((c) => c.kind === "recording" && c.source.mic));
  const { transcribe, cancelJob } = useStore.getState();
  const [model, setModel] = useState(saved ?? "base.en");
  const [language, setLanguage] = useState("");
  const info = speech?.models.find((m) => m.id === model) ?? speech?.models[0];

  if (job) {
    const label = job.stage === "download" ? `Downloading the ${info?.label ?? "speech"} model (once)…` : job.stage === "listen" ? "Transcribing on this computer…" : "Starting…";
    return <JobBar label={label} fraction={job.fraction} onCancel={() => cancelJob("transcribe")} />;
  }
  if (!hasMic) return <p className="hint">This project has no microphone audio, so there is nothing to transcribe.</p>;
  if (speech && !speech.available) return <p className="hint warn">This build of Cinesnap was made without speech recognition.</p>;
  return (
    <>
      <SelectRow label="Model" value={info?.id ?? model} options={(speech?.models ?? []).map((m) => ({ value: m.id, label: `${m.label}${m.downloaded ? "" : ` · ${m.megabytes} MB`}` }))} onChange={setModel} />
      {info && !info.englishOnly && <SelectRow label="Language" value={language} options={LANGUAGES} onChange={setLanguage} />}
      <button className="btn primary wide" onClick={() => void transcribe(info?.id ?? model, info?.englishOnly ? "en" : language || null)}>
        {hasTranscript ? "Transcribe again" : "Generate captions"}
      </button>
      <p className="hint">
        {info?.hint} Speech is recognised on this computer with whisper.cpp; nothing is uploaded.
        {info && !info.downloaded ? ` The model is downloaded once (${info.megabytes} MB) and works offline after that.` : ""}
        {hasTranscript ? " Transcribing again replaces the text and any corrections you made." : ""}
      </p>
    </>
  );
}

function Transcript({ words, onTidy }: { words: Word[]; onTidy: () => void }) {
  const cuts = useStore((s) => s.project!.cuts);
  const sel = useStore((s) => s.wordSel);
  // Index of the word under the playhead; changes a few times a second at most.
  const current = useStore((s) => {
    const t = s.sourceTime();
    let lo = 0;
    let hi = words.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (words[mid].t0 <= t) lo = mid;
      else hi = mid - 1;
    }
    return words.length && words[lo].t0 <= t && t < words[lo].t1 + 250 ? lo : -1;
  });
  const { selectWords, cutWords, restoreWords, editWord, seekSource, setPlaying } = useStore.getState();
  const [editing, setEditing] = useState<number | null>(null);
  const cutFlags = useMemo(() => words.map((w) => isWordCut(w, cuts)), [words, cuts]);
  const [a, b] = sel ?? [-1, -1];
  const selCut = sel ? cutFlags.slice(a, b + 1).every(Boolean) : false;
  const removed = cutFlags.filter(Boolean).length;

  return (
    <>
      <div className="btn-row">
        <button className="btn small" disabled={!sel || selCut} title="Cut the selected words out of the video (Del)" onClick={() => sel && cutWords(a, b)}>
          ✂ Cut words
        </button>
        <button className="btn small" disabled={!sel || !cutFlags.slice(a, b + 1).some(Boolean)} onClick={() => sel && restoreWords(a, b)}>
          Restore
        </button>
        <button className="btn small" onClick={onTidy}>
          Remove fillers & silences…
        </button>
      </div>
      <div className="transcript" onPointerDown={(e) => e.target === e.currentTarget && selectWords(null)}>
        {words.map((w, i) =>
          editing === i ? (
            <input
              key={i}
              className="word-edit"
              autoFocus
              defaultValue={w.text}
              size={Math.max(2, w.text.length)}
              onBlur={(e) => {
                editWord(i, e.target.value);
                setEditing(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
                if (e.key === "Escape") setEditing(null);
                e.stopPropagation();
              }}
            />
          ) : (
            <span
              key={i}
              className={`word ${cutFlags[i] ? "cut" : ""} ${i >= a && i <= b ? "selected" : ""} ${i === current ? "current" : ""}`}
              onClick={(e) => {
                if (e.shiftKey && sel) selectWords([Math.min(a, i), Math.max(b, i)]);
                else {
                  selectWords([i, i]);
                  setPlaying(false);
                  seekSource(w.t0);
                }
              }}
              onDoubleClick={() => setEditing(i)}
            >
              {w.text}
            </span>
          ),
        )}
      </div>
      <p className="hint">
        Click a word to jump to it, Shift-click to select a range, then cut it: the video is cut with it. Double-click a word to correct it.
        {removed ? ` ${removed} of ${words.length} words are cut.` : ""}
      </p>
    </>
  );
}

export function CaptionsPanel({ onTidy }: { onTidy: () => void }) {
  const project = useStore((s) => s.project)!;
  const { update, setError } = useStore.getState();
  const set = (key: string, fn: (p: Project) => void) => update(fn, { key });
  const c = project.captions;
  const words = project.transcript?.words;
  const [exported, setExported] = useState("");

  const exportAs = async (ext: "srt" | "vtt") => {
    try {
      const out = await pickSaveAs("captions", ext.toUpperCase(), [ext]);
      if (!out) return;
      const n = await api.exportCaptions(project, out);
      setExported(`Saved ${n} captions to ${out.split(/[\\/]/).pop()}`);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  return (
    <>
      <Section title="Transcript">
        <Generate hasTranscript={Boolean(words?.length)} />
        {words && words.length > 0 && <Transcript words={words} onTidy={onTidy} />}
        {words && words.length === 0 && <p className="hint">No speech was found in the recording.</p>}
        {!words && (
          <button className="btn wide" onClick={onTidy}>
            Remove long silences…
          </button>
        )}
      </Section>

      {words && words.length > 0 && (
        <Section title="Captions">
          <Toggle label="Show captions in the video" value={c.show} onChange={(v) => set("cap:show", (p) => void (p.captions.show = v))} />
          {c.show && (
            <>
              <div className="stack">
                <span className="row-label">Style</span>
                <Segmented<CaptionSettings["style"]>
                  value={c.style}
                  onChange={(v) => set("cap:style", (p) => void (p.captions.style = v))}
                  options={[
                    { value: "karaoke", label: "Highlight word" },
                    { value: "pop", label: "Word by word" },
                    { value: "plain", label: "Plain" },
                  ]}
                />
              </div>
              <Slider label="Size" value={c.size} min={0.5} max={2.5} step={0.05} format={(v) => `${v.toFixed(2)}×`} onChange={(v) => set("cap:size", (p) => void (p.captions.size = v))} />
              <Slider label="Height on screen" value={c.y} min={0.08} max={0.92} step={0.01} format={(v) => `${Math.round(v * 100)}%`} onChange={(v) => set("cap:y", (p) => void (p.captions.y = v))} />
              <Slider label="Longest line" value={c.maxChars} min={12} max={60} step={1} format={(v) => `${v} characters`} onChange={(v) => set("cap:max", (p) => void (p.captions.maxChars = v))} />
              <SelectRow label="Font" value={c.font ?? ""} options={[{ value: "", label: `Theme font (${project.theme.font})` }, ...FONT_OPTIONS]} onChange={(v) => set("cap:font", (p) => void (p.captions.font = v || undefined))} />
              <Toggle label="ALL CAPS" value={c.uppercase} onChange={(v) => set("cap:upper", (p) => void (p.captions.uppercase = v))} />
              <ColorInput label="Text" value={c.color.slice(0, 7)} onChange={(v) => set("cap:color", (p) => void (p.captions.color = v))} />
              {c.style === "karaoke" && <ColorInput label="Spoken word" value={c.highlight.slice(0, 7)} onChange={(v) => set("cap:hl", (p) => void (p.captions.highlight = v))} />}
              <Toggle label="Background" value={Boolean(c.background)} onChange={(v) => set("cap:bgon", (p) => void (p.captions.background = v ? "#000000a6" : undefined))} />
              {c.background && <AlphaColorInput label="Fill" value={c.background} onChange={(v) => set("cap:bg", (p) => void (p.captions.background = v))} />}
            </>
          )}
          <div className="btn-row">
            <button className="btn small" onClick={() => void exportAs("srt")}>
              Export .srt
            </button>
            <button className="btn small" onClick={() => void exportAs("vtt")}>
              Export .vtt
            </button>
          </div>
          <p className="hint">{exported || "Subtitle files follow the cuts, so they match the exported video."}</p>
        </Section>
      )}
    </>
  );
}
