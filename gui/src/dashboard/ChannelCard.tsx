import { Badge } from "../adw/Badge";
import { Card } from "../adw/Card";
import { MenuIcon } from "../adw/icons";
import { Select } from "../adw/rows";
import type { ChannelStatus, CurveInfo } from "../daemon/types";
import { dutyPercent } from "../daemon/types";
import { CurveSparkline } from "./CurveSparkline";

/** A labelled readout: dim caption over a mono tabular value. */
export function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="text-right">
      <div className="text-[0.82rem] leading-[1.2] text-dim">{label}</div>
      <div className="numeric">{value}</div>
    </div>
  );
}

function SparkPwm({ history, color }: { history: number[]; color: string }) {
  const W = 320;
  const H = 30;
  const pad = 2;
  const n = history.length;
  const x = (i: number) => pad + (n <= 1 ? 0 : (i / (n - 1)) * (W - pad * 2));
  const y = (p: number) => pad + (1 - p / 255) * (H - pad * 2);
  const d = history
    .map((p, i) => `${i === 0 ? "M" : "L"}${x(i).toFixed(1)},${y(p).toFixed(1)}`)
    .join(" ");
  const fill = n > 1 ? `${d} L${x(n - 1).toFixed(1)},${H - pad} L${pad},${H - pad} Z` : "";
  return (
    <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} aria-hidden="true">
      {fill && <path d={fill} fill={color} opacity="0.08" />}
      {d && <path d={d} fill="none" stroke={color} strokeWidth={2} />}
    </svg>
  );
}

/**
 * A Fans-section card: live duty/RPM readouts, recent-pwm sparkline and
 * the curve binding. The ⋮ button opens the channel-properties dialog.
 */
export function ChannelCard({
  name,
  label,
  channel,
  boundCurve,
  curves,
  temps,
  curveNames,
  pwmHistory,
  onSetCurve,
  onProps,
}: {
  name: string;
  label?: string;
  channel: ChannelStatus;
  boundCurve?: string;
  curves: Record<string, CurveInfo>;
  temps: Record<string, number>;
  curveNames: string[];
  pwmHistory: number[];
  onSetCurve: (channel: string, curve: string) => void;
  onProps: () => void;
}) {
  const overriding = channel.mode === "override";
  const curve = boundCurve ? curves[boundCurve] : undefined;
  const sparkColor = overriding ? "var(--color-warning)" : "var(--color-accent)";
  return (
    <Card className="flex flex-col gap-[13px]">
      <header className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <h3 className="numeric m-0 text-[1.18rem] font-bold">{name}</h3>
          <div className="mt-px text-[0.82rem] text-dim">{label ?? ""}</div>
        </div>
        <div className="flex items-center gap-1">
          {overriding ? (
            <Badge tone="warning">
              override{channel.override_remaining_s !== undefined
                ? ` · ${channel.override_remaining_s}s`
                : ""}
            </Badge>
          ) : (
            <Badge>curve</Badge>
          )}
          <button
            type="button"
            onClick={onProps}
            aria-label="Channel properties"
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-button text-dim hover:bg-[var(--flat-hover-fill)] hover:text-ink"
          >
            <MenuIcon />
          </button>
        </div>
      </header>

      <div className="flex items-end justify-between gap-3">
        <div className="flex items-baseline gap-[6px]">
          <span className="numeric text-[46px] font-light leading-[0.9]">
            {dutyPercent(channel.current_pwm)}
          </span>
          <span className="text-[1.18rem] font-light text-dim">%</span>
          <span className="ml-[2px] text-[0.82rem] text-dim">
            duty · pwm {channel.current_pwm}
          </span>
        </div>
        <div className="flex gap-[22px]">
          <Metric label="Fan speed" value={`${channel.rpm.toLocaleString()} RPM`} />
          <Metric label="Target" value={`${dutyPercent(channel.target_pwm)} %`} />
        </div>
      </div>

      <div className="-mx-[2px]" aria-hidden="true">
        <SparkPwm history={pwmHistory} color={sparkColor} />
      </div>

      <div className="flex items-center gap-[10px] border-t border-separator pt-3">
        <span className="w-[34px] shrink-0 text-[0.82rem] text-dim">curve</span>
        <Select
          value={boundCurve ?? ""}
          options={curveNames}
          mono
          ariaLabel={`Curve for ${name}`}
          onChange={(c) => onSetCurve(name, c)}
          className="flex-1"
        />
        {curve?.kind === "graph" && (
          <div className="max-w-[118px] flex-1">
            <CurveSparkline points={curve.points} liveTemp={temps[curve.sensor]} height={30} />
          </div>
        )}
        {curve?.kind === "mix" && (
          <span className="numeric text-[0.82rem] text-dim">
            {curve.function}({curve.members.length})
          </span>
        )}
      </div>
    </Card>
  );
}
