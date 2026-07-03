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
import type { Sample } from "./useDaemonStatus";

// Categorical slots in fixed order (validated for the dark surface):
// sensors are assigned by first appearance, never re-colored.
const SERIES_COLORS = ["#3987e5", "#199e70", "#c98500", "#9085e9"];

const timeOfDay = (at: number) =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

export function TempChart({ history }: { history: Sample[] }) {
  if (history.length === 0) return null;
  const sensors = Object.keys(history[history.length - 1].status.temps);
  const data = history.map((s) => ({ at: s.at, ...s.status.temps }));

  return (
    <ResponsiveContainer width="100%" height={220}>
      <LineChart data={data} margin={{ top: 8, right: 48, bottom: 0, left: -16 }}>
        <CartesianGrid stroke="var(--grid)" vertical={false} />
        <XAxis
          dataKey="at"
          type="number"
          domain={["dataMin", "dataMax"]}
          tickFormatter={timeOfDay}
          stroke="var(--muted)"
          tickLine={false}
          axisLine={{ stroke: "var(--axis)" }}
          minTickGap={60}
        />
        <YAxis
          domain={[20, 100]}
          unit="°"
          stroke="var(--muted)"
          tickLine={false}
          axisLine={false}
        />
        <Tooltip
          contentStyle={{
            background: "var(--surface)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            color: "var(--ink)",
          }}
          labelFormatter={(at) => timeOfDay(at as number)}
          formatter={(value) => [`${(value as number).toFixed(1)} °C`]}
          isAnimationActive={false}
        />
        <Legend wrapperStyle={{ color: "var(--ink-2)" }} iconType="plainline" />
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
