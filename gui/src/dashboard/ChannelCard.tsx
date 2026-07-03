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
    <article className="flex flex-col gap-3 rounded-xl bg-card px-4 py-3.5">
      <header className="flex items-start justify-between gap-2">
        <div>
          <h3 className="font-bold">{name}</h3>
          {label && <span className="text-[13px] text-dim">{label}</span>}
        </div>
        {overriding ? (
          <span
            className="rounded-full bg-warning/15 px-2.5 py-0.5 text-xs font-bold whitespace-nowrap text-warning"
            role="status"
          >
            Override · {channel.override_remaining_s ?? 0}s left
          </span>
        ) : (
          <span className="rounded-full bg-white/10 px-2.5 py-0.5 text-xs whitespace-nowrap text-dim">
            Curve
          </span>
        )}
      </header>
      <div className="flex items-end justify-between gap-3">
        <div>
          <span className="text-4xl leading-none font-light tabular-nums">
            {dutyPercent(channel.current_pwm)}%
          </span>
          <span className="ml-1.5 text-[13px] text-dim">duty</span>
        </div>
        <dl className="flex gap-5">
          <div>
            <dt className="text-xs text-dim">Fan speed</dt>
            <dd className="text-[15px] tabular-nums">
              {channel.rpm.toLocaleString()} RPM
            </dd>
          </div>
          <div>
            <dt className="text-xs text-dim">Curve target</dt>
            <dd className="text-[15px] tabular-nums">
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
              stroke={overriding ? "var(--color-warning)" : "var(--color-accent)"}
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
