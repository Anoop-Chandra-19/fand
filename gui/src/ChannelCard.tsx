import { Line, LineChart, ResponsiveContainer, YAxis } from "recharts";
import type { ChannelStatus } from "./types";
import { dutyPercent } from "./types";
import type { Sample } from "./useDaemonStatus";

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
    <article className={`card${overriding ? " overriding" : ""}`}>
      <header className="card-head">
        <div>
          <h3>{name}</h3>
          {label && <span className="card-label">{label}</span>}
        </div>
        {overriding ? (
          <span className="badge override" role="status">
            ⏱ override · {channel.override_remaining_s ?? 0}s left
          </span>
        ) : (
          <span className="badge curve">curve</span>
        )}
      </header>
      <div className="card-body">
        <div className="stat">
          <span className="stat-value">{dutyPercent(channel.current_pwm)}%</span>
          <span className="stat-caption">duty</span>
        </div>
        <dl className="card-facts">
          <div>
            <dt>fan</dt>
            <dd>{channel.rpm.toLocaleString()} RPM</dd>
          </div>
          <div>
            <dt>curve target</dt>
            <dd>{dutyPercent(channel.target_pwm)}%</dd>
          </div>
        </dl>
      </div>
      <div className="sparkline" aria-hidden="true">
        <ResponsiveContainer width="100%" height={36}>
          <LineChart data={spark} margin={{ top: 2, right: 0, bottom: 2, left: 0 }}>
            <YAxis domain={[0, 255]} hide />
            <Line
              dataKey="pwm"
              stroke={overriding ? "var(--warning)" : "var(--series-1)"}
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
