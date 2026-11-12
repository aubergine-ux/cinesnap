import { useState, type ReactNode } from "react";

export function Section({ title, children, defaultOpen = true, right }: { title: string; children: ReactNode; defaultOpen?: boolean; right?: ReactNode }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <section className={`section ${open ? "open" : ""}`}>
      <header className="section-head" onClick={() => setOpen(!open)}>
        <span className="chev" aria-hidden>
          ›
        </span>
        <h3>{title}</h3>
        <span className="section-right" onClick={(e) => e.stopPropagation()}>
          {right}
        </span>
      </header>
      {open && <div className="section-body">{children}</div>}
    </section>
  );
}

export function Slider({
  label,
  value,
  min,
  max,
  step = 0.01,
  format = (v) => v.toFixed(2),
  onChange,
  disabled,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  const pct = ((value - min) / (max - min)) * 100;
  return (
    <div className={`slider ${disabled ? "disabled" : ""}`}>
      <div className="slider-top">
        <span className="row-label">{label}</span>
        <span className="slider-value">{format(value)}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        style={{ ["--pct" as string]: `${Math.max(0, Math.min(100, pct))}%` }}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

export function Toggle({ label, value, onChange }: { label: string; value: boolean; onChange: (v: boolean) => void }) {
  return (
    <div className="row">
      <span className="row-label">{label}</span>
      <button type="button" role="switch" aria-checked={value} className={`toggle ${value ? "on" : ""}`} onClick={() => onChange(!value)}>
        <span />
      </button>
    </div>
  );
}

export function Segmented<T extends string | number>({
  options,
  value,
  onChange,
  disabled,
}: {
  options: { value: T; label: string }[];
  value: T;
  onChange: (v: T) => void;
  disabled?: boolean;
}) {
  return (
    <div className={`segmented ${disabled ? "disabled" : ""}`} role="radiogroup">
      {options.map((o) => (
        <button key={String(o.value)} type="button" role="radio" aria-checked={o.value === value} disabled={disabled} className={o.value === value ? "active" : ""} onClick={() => onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Select({ value, options, onChange }: { value: string; options: { value: string; label: string }[]; onChange: (v: string) => void }) {
  return (
    <select className="select" value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

export function ColorInput({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <div className="row">
      <span className="row-label">{label}</span>
      <span className="color">
        <input type="color" value={/^#[0-9a-f]{6}$/i.test(value) ? value : "#000000"} onChange={(e) => onChange(e.target.value)} />
        <code>{value}</code>
      </span>
    </div>
  );
}

export function TextInput({ label, value, placeholder, onChange, type = "text" }: { label: string; value: string; placeholder?: string; onChange: (v: string) => void; type?: "text" | "password" | "url" }) {
  return (
    <label className="stack">
      <span className="row-label">{label}</span>
      <input className="text-input" type={type} value={value} placeholder={placeholder} spellCheck={false} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

export function TextArea({ label, value, rows = 2, onChange }: { label: string; value: string; rows?: number; onChange: (v: string) => void }) {
  return (
    <label className="stack">
      <span className="row-label">{label}</span>
      <textarea className="text-input" rows={rows} value={value} spellCheck={false} onChange={(e) => onChange(e.target.value)} />
    </label>
  );
}

export function SelectRow({ label, value, options, onChange }: { label: string; value: string; options: { value: string; label: string }[]; onChange: (v: string) => void }) {
  return (
    <div className="row">
      <span className="row-label">{label}</span>
      <Select value={value} options={options} onChange={onChange} />
    </div>
  );
}

/** Colour with optional transparency (#rrggbbaa keeps its alpha when the hue changes). */
export function AlphaColorInput({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  const rgb = /^#[0-9a-f]{6}/i.test(value) ? value.slice(0, 7) : "#000000";
  const alpha = /^#[0-9a-f]{8}$/i.test(value) ? parseInt(value.slice(7), 16) / 255 : 1;
  const join = (c: string, a: number) => (a >= 0.995 ? c : `${c}${Math.round(a * 255).toString(16).padStart(2, "0")}`);
  return (
    <>
      <ColorInput label={label} value={rgb} onChange={(c) => onChange(join(c, alpha))} />
      <Slider label={`${label} opacity`} value={alpha} min={0.1} max={1} step={0.01} format={(v) => `${Math.round(v * 100)}%`} onChange={(a) => onChange(join(rgb, a))} />
    </>
  );
}

/** A progress bar for a background job, with a cancel button. */
export function JobBar({ label, fraction, onCancel }: { label: string; fraction: number; onCancel?: () => void }) {
  return (
    <div className="job">
      <div className="job-top">
        <span>{label}</span>
        <span className="slider-value">{Math.round(fraction * 100)}%</span>
        {onCancel && (
          <button className="btn small" onClick={onCancel}>
            Cancel
          </button>
        )}
      </div>
      <div className="progress">
        <span style={{ width: `${Math.max(0, Math.min(1, fraction)) * 100}%` }} />
      </div>
    </div>
  );
}
