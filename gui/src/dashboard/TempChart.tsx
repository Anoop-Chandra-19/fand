import type { Sample } from "../daemon/types";

export const SERIES = [
  "var(--color-series-1)",
  "var(--color-series-2)",
  "var(--color-series-3)",
  "var(--color-series-4)",
];

/**
 * The Temperatures card: every sensor's rolling history as a GNOME-palette
 * line chart, with a legend of live readings. Series colors are assigned
 * by first appearance (the daemon's sensor order is stable) and a sensor
 * is never re-colored.
 */
export function TempChartCard({
  history,
  sensors,
  labels,
  temps,
}: {
  history: Sample[];
  sensors: string[];
  labels: Record<string, string>;
  temps: Record<string, number>;
}) {
  return (
    <div className="rounded-card bg-card px-4 pb-2 pt-[14px] shadow-card">
      <Chart history={history} sensors={sensors} />
      <div className="mt-[6px] border-t border-separator pt-2">
        <div className="flex flex-wrap gap-6 px-[2px] pb-[2px] pt-1">
          {sensors.map((s, si) => (
            <div key={s} className="flex min-w-0 flex-[1_1_260px] items-center gap-[9px]">
              <span
                className="h-[9px] w-[9px] shrink-0 rounded-[3px]"
                style={{ background: SERIES[si % SERIES.length] }}
              />
              <div className="overflow-hidden text-ellipsis whitespace-nowrap text-[0.82rem] text-dim">
                {labels[s] ?? s}
              </div>
              <span
                className="numeric ml-auto text-[1.18rem] font-light"
                style={{ color: SERIES[si % SERIES.length] }}
              >
                {temps[s]?.toFixed(1) ?? "—"}
                <span className="text-[0.82rem] opacity-70"> °C</span>
              </span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

function Chart({ history, sensors }: { history: Sample[]; sensors: string[] }) {
  const W = 1000;
  const H = 200;
  const PADL = 30;
  const PADR = 16;
  const PADT = 12;
  const PADB = 22;
  const iw = W - PADL - PADR;
  const ih = H - PADT - PADB;

  // Base domain 20–100 °C, widened in 10° steps when readings leave it —
  // an excursion toward the 115 °C failsafe must stay visible, never sit
  // pinned to the frame edge.
  let tMin = 20;
  let tMax = 100;
  for (const h of history) {
    for (const s of sensors) {
      const t = h.status.temps[s];
      if (t === undefined) continue;
      tMin = Math.min(tMin, Math.floor(t / 10) * 10);
      tMax = Math.max(tMax, Math.ceil(t / 10) * 10);
    }
  }
  const ticks: number[] = [];
  for (let t = tMin; t <= tMax; t += 20) ticks.push(t);

  // The x axis is real time, so a disconnect gap keeps its width instead
  // of silently compressing the timeline.
  const t0 = history[0]?.at ?? 0;
  const span = history.length > 1 ? history[history.length - 1].at - t0 : 0;
  const x = (at: number) => PADL + (span === 0 ? 0 : ((at - t0) / span) * iw);
  const y = (t: number) => PADT + (1 - (t - tMin) / (tMax - tMin)) * ih;

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      width="100%"
      height={H}
      role="img"
      aria-label="Temperatures"
      className="block"
    >
      {ticks.map((t) => (
        <g key={t}>
          <line x1={PADL} y1={y(t)} x2={W - PADR} y2={y(t)} stroke="var(--color-separator)" />
          <text
            x={PADL - 6}
            y={y(t) + 3.5}
            fontSize="10"
            fill="var(--color-dim)"
            textAnchor="end"
            style={{ fontVariantNumeric: "tabular-nums", fontFamily: "var(--font-mono)" }}
          >
            {t}°
          </text>
        </g>
      ))}
      {sensors.map((s, si) => {
        const temps = history
          .map((h) => [h.at, h.status.temps[s]] as const)
          .filter((pair): pair is readonly [number, number] => pair[1] !== undefined);
        if (temps.length === 0) return null;
        const path = temps
          .map(([at, t], k) => `${k === 0 ? "M" : "L"}${x(at).toFixed(1)},${y(t).toFixed(1)}`)
          .join(" ");
        const last = temps[temps.length - 1];
        const fill =
          temps.length > 1
            ? `${path} L${x(last[0]).toFixed(1)},${(H - PADB).toFixed(1)} L${x(temps[0][0]).toFixed(1)},${(H - PADB).toFixed(1)} Z`
            : "";
        const color = SERIES[si % SERIES.length];
        return (
          <g key={s}>
            {fill && <path d={fill} fill={color} opacity="0.07" />}
            <path d={path} fill="none" stroke={color} strokeWidth={2} strokeLinejoin="round" />
            <circle cx={x(last[0])} cy={y(last[1])} r={3} fill={color} />
          </g>
        );
      })}
    </svg>
  );
}
