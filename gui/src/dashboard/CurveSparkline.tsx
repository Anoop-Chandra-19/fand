import { useId } from "react";
import type { CurvePoint } from "../daemon/types";
import { interpolate } from "../daemon/eval";
import { dutyPercent } from "../daemon/types";

/**
 * Fan-curve preview: accent polyline + soft area fill, clamped flat to
 * both edges to match the daemon's endpoint hold. The 20–100 °C base
 * domain widens (in 10° steps) to fit any point outside it — points
 * beyond 100 °C are legal config and must not be clipped. Optionally
 * overlays the live source temperature as a warning-colored marker with
 * the current output %; an off-domain reading parks at the frame edge
 * (its output % stays exact — the curve is flat past its endpoints).
 */
export function CurveSparkline({
  points,
  liveTemp,
  width = 300,
  height = 90,
  grid = false,
  showPoints = false,
}: {
  points: CurvePoint[];
  liveTemp?: number;
  width?: number;
  height?: number;
  grid?: boolean;
  showPoints?: boolean;
}) {
  let tempMin = 20;
  let tempMax = 100;
  for (const [t] of points) {
    tempMin = Math.min(tempMin, Math.floor(t / 10) * 10);
    tempMax = Math.max(tempMax, Math.ceil(t / 10) * 10);
  }
  const pad = 4;
  const iw = width - pad * 2;
  const ih = height - pad * 2;
  const x = (t: number) => pad + ((t - tempMin) / (tempMax - tempMin)) * iw;
  const y = (p: number) => pad + (1 - p / 255) * ih;

  const path = points.length
    ? `M${x(tempMin).toFixed(1)},${y(points[0][1]).toFixed(1)} ` +
      points.map(([t, p]) => `L${x(t).toFixed(1)},${y(p).toFixed(1)}`).join(" ") +
      ` L${x(tempMax).toFixed(1)},${y(points[points.length - 1][1]).toFixed(1)}`
    : "";
  const area = path
    ? `${path} L${x(tempMax).toFixed(1)},${(height - pad).toFixed(1)} L${x(tempMin).toFixed(1)},${(height - pad).toFixed(1)} Z`
    : "";

  const showLive = liveTemp !== undefined;
  const lx = showLive ? x(Math.min(tempMax, Math.max(tempMin, liveTemp))) : 0;
  const ly = showLive ? y(interpolate(points, liveTemp)) : 0;
  const liveOut = showLive ? dutyPercent(interpolate(points, liveTemp)) : 0;
  const pillW = 34;
  const pillX = Math.min(Math.max(lx + 6, pad), width - pad - pillW);
  const gradId = useId().replace(/:/g, "");

  return (
    <svg viewBox={`0 0 ${width} ${height}`} width="100%" height={height} aria-hidden="true">
      <defs>
        <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--color-accent)" stopOpacity="0.24" />
          <stop offset="100%" stopColor="var(--color-accent)" stopOpacity="0" />
        </linearGradient>
      </defs>
      {grid &&
        [0.25, 0.5, 0.75].map((f) => (
          <line
            key={f}
            x1={pad}
            y1={pad + f * ih}
            x2={width - pad}
            y2={pad + f * ih}
            stroke="var(--color-separator)"
            strokeWidth="1"
          />
        ))}
      {area && <path d={area} fill={`url(#${gradId})`} />}
      {path && (
        <path
          d={path}
          fill="none"
          stroke="var(--color-accent)"
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
      )}
      {showPoints &&
        points.map(([t, p], i) => (
          <circle
            key={i}
            cx={x(t)}
            cy={y(p)}
            r={2.6}
            fill="var(--color-accent)"
            stroke="var(--color-view)"
            strokeWidth={1.5}
          />
        ))}
      {showLive && (
        <>
          <line
            x1={lx}
            y1={pad}
            x2={lx}
            y2={height - pad}
            stroke="var(--color-warning)"
            strokeDasharray="3,3"
            opacity="0.6"
          />
          <circle
            cx={lx}
            cy={ly}
            r={5}
            fill="var(--color-warning)"
            stroke="var(--color-view)"
            strokeWidth={1.5}
          />
          <g
            transform={`translate(${pillX}, ${Math.max(pad + 9, Math.min(ly - 9, height - pad - 18))})`}
          >
            <rect width={pillW} height={16} rx={8} fill="var(--color-warning)" />
            <text
              x={pillW / 2}
              y={11.5}
              textAnchor="middle"
              fontSize="10"
              fontWeight="700"
              fill="#26210a"
              style={{ fontVariantNumeric: "tabular-nums" }}
            >
              {liveOut}%
            </text>
          </g>
        </>
      )}
    </svg>
  );
}
