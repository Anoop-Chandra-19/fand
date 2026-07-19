import { useState } from "react";
import { CloseButton, Dialog } from "../adw/Dialog";
import { BoxedList, ComboRow, SpinRow } from "../adw/rows";
import type { ChannelSettings, WriteResult } from "../daemon/types";
import { dutyPercent } from "../daemon/types";

/**
 * Channel properties — boxed-list rows with instant apply through the
 * daemon's validation. min_pwm is hard-floored per the safety invariants:
 * 60 everywhere, 80 on pwm1 (the AIO pump rides that header). There are
 * deliberately no zero-RPM controls — fans never stop.
 */
export function ChannelPropsDialog({
  name,
  label,
  settings,
  boundCurve,
  curveNames,
  setChannelCurve,
  setMinPwm,
  setSmoothingSeconds,
  setOffsetPwm,
  onClose,
}: {
  name: string;
  label?: string;
  settings: ChannelSettings;
  boundCurve?: string;
  curveNames: string[];
  setChannelCurve: (channel: string, curve: string) => Promise<WriteResult>;
  setMinPwm: (channel: string, minPwm: number) => Promise<WriteResult>;
  setSmoothingSeconds: (channel: string, seconds: number) => Promise<WriteResult>;
  setOffsetPwm: (channel: string, offset: number) => Promise<WriteResult>;
  onClose: () => void;
}) {
  // One note line: errors in red, applied-with-caveat warnings in the
  // dim style — a warning is a success, it must not read as a failure.
  const [note, setNote] = useState<{ error: boolean; text: string } | null>(null);
  const report = ({ error, warning }: WriteResult) => {
    setNote(
      error
        ? { error: true, text: error }
        : warning
          ? { error: false, text: warning }
          : null,
    );
  };
  const isPump = name === "pwm1";
  const floor = isPump ? 80 : 60;

  return (
    <Dialog width={468} label={`${name} channel properties`} onClose={onClose}>
      <div className="flex shrink-0 items-center justify-between gap-2 border-b border-separator py-2.5 pl-4 pr-3">
        <div>
          <div className="font-mono font-bold">{name}</div>
          {label && <div className="text-[0.82rem] text-dim">{label}</div>}
        </div>
        <CloseButton onClose={onClose} />
      </div>
      <div className="flex flex-col gap-3 overflow-auto p-4">
        <BoxedList>
          <ComboRow
            title="Curve"
            value={boundCurve ?? ""}
            options={curveNames}
            onChange={(c) => void setChannelCurve(name, c).then(report)}
          />
          <SpinRow
            title="Min PWM"
            subtitle={`${isPump ? "pump inline — floor 80" : "hard floor 60"} · = ${dutyPercent(settings.min_pwm)} % duty`}
            value={settings.min_pwm}
            min={floor}
            max={255}
            unit="pwm"
            onChange={(v) => void setMinPwm(name, v).then(report)}
          />
          <SpinRow
            title="Smoothing"
            subtitle="rolling average over this window"
            value={settings.smoothing_seconds}
            min={1}
            max={60}
            unit="s"
            onChange={(v) => void setSmoothingSeconds(name, v).then(report)}
          />
          <SpinRow
            title="Curve offset"
            subtitle="bias added before the floor"
            value={settings.offset_pwm}
            min={-60}
            max={60}
            unit="pwm"
            onChange={(v) => void setOffsetPwm(name, v).then(report)}
          />
        </BoxedList>
        {note && (
          <div
            className={`px-1 text-[0.82rem] leading-[1.4] ${note.error ? "text-error" : "text-dim"}`}
          >
            {note.error ? note.text : `Applied — ${note.text}`}
          </div>
        )}
        <div className="px-1 text-[0.82rem] leading-[1.4] text-dim">
          Changes apply instantly through the daemon's validation.
        </div>
      </div>
    </Dialog>
  );
}
