import { useEffect, useState } from "react";
import type { ChannelSettingsPayload } from "../daemon/types";

type WriteResult = Promise<string | null>;

interface Props {
  data: ChannelSettingsPayload | null;
  labels: Record<string, string>;
  setMinPwm: (channel: string, minPwm: number) => WriteResult;
  setSmoothingSeconds: (channel: string, seconds: number) => WriteResult;
}

interface NumberFieldProps {
  label: string;
  value: number;
  min: number;
  max?: number;
  onCommit: (value: number) => WriteResult;
}

function NumberField({ label, value, min, max, onCommit }: NumberFieldProps) {
  const [draft, setDraft] = useState(String(value));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => setDraft(String(value)), [value]);

  const commit = async () => {
    const parsed = Number(draft);
    if (!Number.isFinite(parsed)) {
      setDraft(String(value));
      return;
    }
    const clamped = Math.min(max ?? parsed, Math.max(min, Math.round(parsed)));
    const err = await onCommit(clamped);
    if (err) {
      setError(err);
      setDraft(String(value));
    } else {
      setError(null);
      setDraft(String(clamped));
    }
  };

  return (
    <div className="flex items-center justify-between gap-3 text-[13px]">
      <span className="text-dim">{label}</span>
      <div className="flex flex-col items-end gap-0.5">
        <input
          type="number"
          value={draft}
          min={min}
          max={max}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => void commit()}
          onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          className="w-20 rounded-md bg-white/10 px-2 py-1 text-right outline-none focus:ring-1 focus:ring-accent"
        />
        {error && <span className="text-[11px] text-error">{error}</span>}
      </div>
    </div>
  );
}

export function SettingsPage({ data, labels, setMinPwm, setSmoothingSeconds }: Props) {
  if (!data) {
    return <p className="py-12 text-center text-dim">Loading settings…</p>;
  }

  return (
    <section className="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-5">
      {Object.entries(data).map(([channel, settings]) => {
        // pwm1 carries the Arctic Liquid Freezer II 360's pump inline —
        // min_pwm must never drop below its proven-safe floor of 80. The
        // daemon enforces both floors too; the clamp here just saves a
        // round-trip.
        const isPump = channel === "pwm1";
        return (
          <article key={channel} className="flex flex-col gap-3 rounded-xl bg-card px-5 py-4">
            <header>
              <h3 className="font-bold">{channel}</h3>
              {labels[channel] && <span className="text-[13px] text-dim">{labels[channel]}</span>}
            </header>
            <NumberField
              label="Min duty (0–255)"
              value={settings.min_pwm}
              min={isPump ? 80 : 60}
              max={255}
              onCommit={(v) => setMinPwm(channel, v)}
            />
            <NumberField
              label="Smoothing (s)"
              value={settings.smoothing_seconds}
              min={1}
              onCommit={(v) => setSmoothingSeconds(channel, v)}
            />
            {/* Read-only until the phase-10 editor gains a control. */}
            {settings.offset_pwm !== 0 && (
              <div className="flex items-center justify-between gap-3 text-[13px]">
                <span className="text-dim">Curve offset</span>
                <span>
                  {settings.offset_pwm > 0 ? `+${settings.offset_pwm}` : settings.offset_pwm}
                </span>
              </div>
            )}
          </article>
        );
      })}
    </section>
  );
}
