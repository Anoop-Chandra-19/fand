import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import type { Sample } from "../daemon/types";

// GNOME palette slots in fixed order (CVD-validated against the card
// surface): sensors are assigned by first appearance, never re-colored.
const SERIES_COLORS = ["#62a0ea", "#33d17a", "#ffa348", "#dc8add"];

const timeOfDay = (at: number) =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

export function TempChart({ history }: { history: Sample[] }) {
  if (history.length === 0) return null;
  const sensors = Object.keys(history[history.length - 1].status.temps);
  const data = history.map((s) => ({ at: s.at, ...s.status.temps }));

  return (
    <ResponsiveContainer width="100%" height={220}>
      <LineChart data={data} margin={{ top: 8, right: 48, bottom: 0, left: -16 }}>
        <CartesianGrid stroke="var(--color-separator)" vertical={false} />
        <XAxis
          dataKey="at"
          type="number"
          domain={["dataMin", "dataMax"]}
          tickFormatter={timeOfDay}
          stroke="var(--color-dim)"
          tickLine={false}
          axisLine={{ stroke: "var(--color-separator)" }}
          minTickGap={60}
        />
        <YAxis
          domain={[20, 100]}
          unit="°"
          stroke="var(--color-dim)"
          tickLine={false}
          axisLine={false}
        />
        <Tooltip
          contentStyle={{
            background: "var(--color-popover)",
            border: "none",
            borderRadius: 12,
            boxShadow: "0 2px 8px rgb(0 0 0 / 0.4)",
            color: "var(--color-ink)",
          }}
          labelFormatter={(at) => timeOfDay(at as number)}
          formatter={(value) => [`${(value as number).toFixed(1)} °C`]}
          isAnimationActive={false}
        />
        <Legend wrapperStyle={{ color: "var(--color-dim)" }} iconType="plainline" />
        {sensors.map((name, i) => (
          <Line
            key={name}
            dataKey={name}
            stroke={SERIES_COLORS[i % SERIES_COLORS.length]}
            strokeWidth={2}
            dot={false}
            isAnimationActive={false}
            label={(props: { index?: number; x?: string | number; y?: string | number }) =>
              props.index === data.length - 1 ? (
                <text
                  x={Number(props.x ?? 0) + 6}
                  y={Number(props.y ?? 0)}
                  fill={SERIES_COLORS[i % SERIES_COLORS.length]}
                  fontSize={12}
                  dominantBaseline="middle"
                >
                  {name}
                </text>
              ) : (
                <g />
              )
            }
          />
        ))}
      </LineChart>
    </ResponsiveContainer>
  );
}
