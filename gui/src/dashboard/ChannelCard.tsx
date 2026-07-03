import { Line, LineChart, ResponsiveContainer, YAxis } from "recharts";
import type { ChannelStatus, Sample } from "../daemon/types";
import { dutyPercent } from "../daemon/types";

interface Props {
  name: string;
  label?: string;
  channel: ChannelStatus;
  history: Sample[];
}

export function ChannelCard({ name, label, channel, history }: Props) {
  const overriding = channel.mode === "override";
  const spark = history.map((s) => ({
    at: s.at,
    pwm: s.status.channels[name]?.current_pwm ?? null,
  }));

  return (
    <article
      className={`flex flex-col gap-2.5 rounded-[10px] border bg-surface px-4 py-3.5 ${
        overriding ? "border-warning" : "border-white/10"
      }`}
    >
      <header className="flex items-start justify-between gap-2">
        <div>
          <h3 className="font-semibold">{name}</h3>
          {label && <span className="text-xs text-muted">{label}</span>}
        </div>
        {overriding ? (
          <span
            className="rounded-full bg-warning px-2 py-0.5 text-xs font-bold whitespace-nowrap text-surface"
            role="status"
          >
            ⏱ override · {channel.override_remaining_s ?? 0}s left
          </span>
        ) : (
          <span className="rounded-full border border-white/10 px-2 py-0.5 text-xs whitespace-nowrap text-ink-2">
            curve
          </span>
        )}
      </header>
      <div className="flex items-end justify-between gap-3">
        <div>
          <span className="text-4xl leading-none font-bold">
            {dutyPercent(channel.current_pwm)}%
          </span>
          <span className="ml-1.5 text-xs text-muted">duty</span>
        </div>
        <dl className="flex gap-4">
          <div>
            <dt className="text-[0.72rem] tracking-wider text-muted uppercase">fan</dt>
            <dd className="text-[0.95rem] tabular-nums">
              {channel.rpm.toLocaleString()} RPM
            </dd>
          </div>
          <div>
            <dt className="text-[0.72rem] tracking-wider text-muted uppercase">
              curve target
            </dt>
            <dd className="text-[0.95rem] tabular-nums">
              {dutyPercent(channel.target_pwm)}%
            </dd>
          </div>
        </dl>
      </div>
      <div className="-mx-1.5 -mb-1.5" aria-hidden="true">
        <ResponsiveContainer width="100%" height={36}>
          <LineChart data={spark} margin={{ top: 2, right: 0, bottom: 2, left: 0 }}>
            <YAxis domain={[0, 255]} hide />
            <Line
              dataKey="pwm"
              stroke={overriding ? "var(--color-warning)" : "var(--color-series-1)"}
              strokeWidth={2}
              dot={false}
              isAnimationActive={false}
            />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </article>
  );
}
