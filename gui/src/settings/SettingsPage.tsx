import { useEffect, useState } from "react";
import type { ChannelSettings, ChannelSettingsPayload } from "../daemon/types";

type WriteResult = Promise<string | null>;

interface Props {
  data: ChannelSettingsPayload | null;
  labels: Record<string, string>;
  setMinPwm: (channel: string, minPwm: number) => WriteResult;
  setSmoothingSeconds: (channel: string, seconds: number) => WriteResult;
  setZeroRpm: (
    channel: string,
    zeroRpm: boolean,
    kickPwm: number | null,
    kickSeconds: number | null,
  ) => WriteResult;
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

interface ZeroRpmProps {
  channel: string;
  settings: ChannelSettings;
  setZeroRpm: Props["setZeroRpm"];
}

function ZeroRpmSection({ channel, settings, setZeroRpm }: ZeroRpmProps) {
  const [enabling, setEnabling] = useState(false);
  const [kickPwm, setKickPwm] = useState(String(settings.kick_pwm ?? 100));
  const [kickSeconds, setKickSeconds] = useState(String(settings.kick_seconds ?? 3));
  const [error, setError] = useState<string | null>(null);

  const handleEnable = async () => {
    const pwm = Number(kickPwm);
    const secs = Number(kickSeconds);
    if (!Number.isFinite(pwm) || !Number.isFinite(secs) || pwm <= 0 || secs <= 0) {
      setError("enter a kick pwm and seconds above 0");
      return;
    }
    const err = await setZeroRpm(channel, true, pwm, secs);
    if (err) {
      setError(err);
    } else {
      setEnabling(false);
      setError(null);
    }
  };

  const handleDisable = async () => {
    const err = await setZeroRpm(channel, false, null, null);
    setError(err);
  };

  if (settings.zero_rpm) {
    return (
      <div className="flex flex-col gap-1.5 border-t border-separator pt-2.5 text-[13px]">
        <div className="flex items-center justify-between">
          <span className="text-dim">Zero-RPM</span>
          <button
            type="button"
            onClick={() => void handleDisable()}
            className="text-xs text-dim hover:text-error"
          >
            disable
          </button>
        </div>
        <p className="text-dim">
          kicks to {settings.kick_pwm} for {settings.kick_seconds}s when leaving 0
        </p>
        {error && <p className="text-xs text-error">{error}</p>}
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-1.5 border-t border-separator pt-2.5 text-[13px]">
      {enabling ? (
        <>
          <div className="flex items-center gap-2">
            <span className="w-16 shrink-0 text-dim">kick pwm</span>
            <input
              type="number"
              value={kickPwm}
              onChange={(e) => setKickPwm(e.target.value)}
              className="w-20 rounded-md bg-white/10 px-2 py-1 text-right outline-none focus:ring-1 focus:ring-accent"
            />
          </div>
          <div className="flex items-center gap-2">
            <span className="w-16 shrink-0 text-dim">kick secs</span>
            <input
              type="number"
              value={kickSeconds}
              onChange={(e) => setKickSeconds(e.target.value)}
              className="w-20 rounded-md bg-white/10 px-2 py-1 text-right outline-none focus:ring-1 focus:ring-accent"
            />
          </div>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => void handleEnable()}
              className="text-xs text-accent hover:underline"
            >
              enable
            </button>
            <button
              type="button"
              onClick={() => {
                setEnabling(false);
                setError(null);
              }}
              className="text-xs text-dim hover:text-ink"
            >
              cancel
            </button>
          </div>
        </>
      ) : (
        <button
          type="button"
          onClick={() => setEnabling(true)}
          className="self-start text-dim hover:text-ink"
        >
          + enable zero-RPM
        </button>
      )}
      {error && <p className="text-xs text-error">{error}</p>}
    </div>
  );
}

export function SettingsPage({ data, labels, setMinPwm, setSmoothingSeconds, setZeroRpm }: Props) {
  if (!data) {
    return <p className="py-12 text-center text-dim">Loading settings…</p>;
  }

  return (
    <section className="grid grid-cols-[repeat(auto-fit,minmax(300px,1fr))] gap-5">
      {Object.entries(data).map(([channel, settings]) => {
        // pwm1 carries the Arctic Liquid Freezer II 360's pump inline —
        // zero_rpm is never safe here, and min_pwm must never drop below
        // its proven-safe floor of 80.
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
              min={isPump ? 80 : 0}
              max={255}
              onCommit={(v) => setMinPwm(channel, v)}
            />
            <NumberField
              label="Smoothing (s)"
              value={settings.smoothing_seconds}
              min={1}
              onCommit={(v) => setSmoothingSeconds(channel, v)}
            />
            {!isPump && (
              <ZeroRpmSection channel={channel} settings={settings} setZeroRpm={setZeroRpm} />
            )}
          </article>
        );
      })}
    </section>
  );
}
