import { useState, type ReactNode } from "react";
import { CloseButton, Dialog } from "../adw/Dialog";
import { ActionRow, BoxedList, ComboRow, SpinRow } from "../adw/rows";
import { dutyPercent } from "../daemon/types";
import { ACCENTS, type Accent } from "../shell/accent";

function PrefGroup({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children?: ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2">
      <div className="px-1">
        <h2 className="m-0 font-bold">{title}</h2>
        {description && (
          <p className="mb-0 mt-0.5 text-[0.82rem] leading-[1.4] text-dim">{description}</p>
        )}
      </div>
      {children}
    </section>
  );
}

const dimValue = "text-[0.82rem] text-dim";

/**
 * App-level preferences. Daemon behavior is tuned in the per-channel and
 * per-curve dialogs; the failsafe limits are invariants and are shown
 * here read-only so the dialog documents what the daemon enforces.
 */
export function PreferencesDialog({
  accent,
  onAccent,
  chartMinutes,
  onChartMinutes,
  socketPath,
  connected,
  onReloadConfig,
  onClose,
}: {
  accent: Accent;
  onAccent: (accent: Accent) => void;
  chartMinutes: number;
  onChartMinutes: (minutes: number) => void;
  socketPath: string | null;
  connected: boolean | null;
  onReloadConfig: () => Promise<string | null>;
  onClose: () => void;
}) {
  const [reloadError, setReloadError] = useState<string | null>(null);
  return (
    <Dialog width={560} label="Preferences" onClose={onClose}>
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-separator py-2.5 pl-4 pr-3">
        <div className="font-bold">Preferences</div>
        <CloseButton onClose={onClose} />
      </div>
      <div className="flex flex-col gap-5.5 overflow-auto px-5.5 pb-6.5 pt-5">
        <PrefGroup
          title="Appearance"
          description="fand is dark-only, matching its libadwaita styling. Accent tints suggested buttons, switches and curve lines — the cpu/gpu chart series and the warning marker keep their fixed colors."
        >
          <BoxedList>
            <ComboRow
              title="Accent color"
              value={accent}
              options={[...ACCENTS]}
              onChange={(v) => onAccent(v as Accent)}
            />
          </BoxedList>
        </PrefGroup>

        <PrefGroup
          title="Overview"
          description="How much history the Temperatures chart and the per-fan sparklines keep."
        >
          <BoxedList>
            <SpinRow
              title="Chart history"
              value={chartMinutes}
              min={5}
              max={30}
              step={5}
              unit="min"
              onChange={onChartMinutes}
            />
          </BoxedList>
        </PrefGroup>

        <PrefGroup
          title="Daemon"
          description="fand runs as a system service; this window is just a client on its socket."
        >
          <BoxedList>
            <ActionRow
              title="Connection"
              trailing={
                <span
                  className={`numeric text-[0.82rem] ${connected ? "text-success" : "text-error"}`}
                >
                  {connected ? "connected" : "unreachable"}
                </span>
              }
            />
            <ActionRow
              title="Socket"
              trailing={<span className="numeric text-[0.82rem] text-dim">{socketPath ?? "—"}</span>}
            />
            <ActionRow
              title="Reload config from disk"
              subtitle="re-read the daemon's config file and hot-apply it"
              activatable
              onClick={() => {
                void onReloadConfig().then(setReloadError);
              }}
              trailing={
                <span className="text-[0.82rem] font-bold text-accent">Reload</span>
              }
            />
          </BoxedList>
          {reloadError && (
            <div className="px-1 text-[0.82rem] leading-[1.4] text-error">{reloadError}</div>
          )}
        </PrefGroup>

        <PrefGroup
          title="Safety"
          description="The daemon owns the thermal failsafe. These are invariants, not settings — they cannot be disabled or tuned from any client."
        >
          <BoxedList>
            <ActionRow
              title="Failsafe temperature"
              subtitle="sensor failure, ≤ 0 °C or ≥ this → every fan to 100 %, then firmware auto"
              trailing={<span className={`numeric ${dimValue}`}>115 °C</span>}
            />
            <ActionRow
              title="Min PWM floors"
              subtitle={`fans never stop; pwm1 carries the AIO pump inline · = ${dutyPercent(60)}/${dutyPercent(80)} % duty`}
              trailing={<span className={`numeric ${dimValue}`}>60 · 80 on pwm1</span>}
            />
            <ActionRow
              title="On exit"
              subtitle="every exit path hands the fans back to the BIOS"
              trailing={<span className={dimValue}>firmware auto</span>}
            />
          </BoxedList>
        </PrefGroup>
      </div>
    </Dialog>
  );
}
