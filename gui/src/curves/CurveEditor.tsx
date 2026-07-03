import { useRef, useState } from "react";
import type { PointerEvent as ReactPointerEvent } from "react";
import type { CurvePoint } from "../daemon/types";
import {
  PWM_MAX,
  TEMP_MAX,
  TEMP_MIN,
  interpolate,
  pwmToY,
  tempToX,
  xToTemp,
  yToPwm,
} from "./scales";

interface Props {
  points: CurvePoint[];
  /** Current sensor reading this curve is evaluated against, if known. */
  liveTemp?: number;
  /**
   * Fires on drag release and on add/remove — never mid-drag. Returns an
   * error string on failure (the draft then reverts to `points`), or null
   * on success.
   */
  onCommit?: (points: CurvePoint[]) => Promise<string | null>;
}

const WIDTH = 320;
const HEIGHT = 140;
const PAD = 10;
const INNER_W = WIDTH - PAD * 2;
const INNER_H = HEIGHT - PAD * 2;
const MIN_TEMP_GAP = 1;

function project([temp, pwm]: CurvePoint): [number, number] {
  return [PAD + tempToX(temp, INNER_W), PAD + pwmToY(pwm, INNER_H)];
}

function unproject(x: number, y: number): CurvePoint {
  const temp = Math.round(xToTemp(x - PAD, INNER_W));
  const pwm = Math.round(yToPwm(y - PAD, INNER_H));
  return [temp, pwm];
}

function clampPwm(pwm: number): number {
  return Math.min(Math.max(pwm, 0), PWM_MAX);
}

/** Keeps point `i` strictly between its neighbors' temps and on-screen. */
function clampAt(points: CurvePoint[], i: number, temp: number, pwm: number): CurvePoint {
  const lower = i === 0 ? TEMP_MIN : points[i - 1][0] + MIN_TEMP_GAP;
  const upper = i === points.length - 1 ? TEMP_MAX : points[i + 1][0] - MIN_TEMP_GAP;
  const bounded = Math.min(Math.max(temp, lower), Math.max(lower, upper));
  return [Math.round(bounded), clampPwm(pwm)];
}

function svgPoint(svg: SVGSVGElement, clientX: number, clientY: number): [number, number] {
  const pt = svg.createSVGPoint();
  pt.x = clientX;
  pt.y = clientY;
  const ctm = svg.getScreenCTM();
  if (!ctm) return [0, 0];
  const local = pt.matrixTransform(ctm.inverse());
  return [local.x, local.y];
}

export function CurveEditor({ points, liveTemp, onCommit }: Props) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [draft, setDraft] = useState(points);
  const [dragging, setDragging] = useState<number | null>(null);
  const [hovered, setHovered] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Re-seed the draft whenever the confirmed points change underneath us
  // (e.g. a successful write's daemon-confirmed refetch), but not on our
  // own in-flight edits.
  const seeded = useRef(points);
  if (seeded.current !== points) {
    seeded.current = points;
    if (dragging === null) setDraft(points);
  }

  const commit = async (next: CurvePoint[]) => {
    setDraft(next);
    if (!onCommit) return;
    const err = await onCommit(next);
    if (err) {
      setDraft(points);
      setError(err);
    } else {
      setError(null);
    }
  };

  const handlePointerDown = (i: number) => (e: ReactPointerEvent<SVGCircleElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    setDragging(i);
  };

  const handlePointerMove = (i: number) => (e: ReactPointerEvent<SVGCircleElement>) => {
    if (dragging !== i || !svgRef.current) return;
    const [x, y] = svgPoint(svgRef.current, e.clientX, e.clientY);
    const [temp, pwm] = unproject(x, y);
    const next = [...draft];
    next[i] = clampAt(draft, i, temp, pwm);
    setDraft(next);
  };

  const handlePointerUp = (e: ReactPointerEvent<SVGCircleElement>) => {
    e.currentTarget.releasePointerCapture(e.pointerId);
    setDragging(null);
    void commit(draft);
  };

  const handleAddPoint = (e: ReactPointerEvent<SVGRectElement>) => {
    if (!svgRef.current) return;
    const [x, y] = svgPoint(svgRef.current, e.clientX, e.clientY);
    const [temp, pwm] = unproject(x, y);
    const index = draft.findIndex(([t]) => t > temp);
    const insertAt = index === -1 ? draft.length : index;
    const withPlaceholder = [...draft];
    withPlaceholder.splice(insertAt, 0, [temp, clampPwm(pwm)]);
    withPlaceholder[insertAt] = clampAt(withPlaceholder, insertAt, temp, pwm);
    void commit(withPlaceholder);
  };

  const handleRemovePoint = (i: number) => {
    if (draft.length <= 2) return;
    void commit(draft.filter((_, idx) => idx !== i));
  };

  const path = draft
    .map(project)
    .map(([x, y], i) => `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`)
    .join(" ");

  const showLive =
    liveTemp !== undefined && liveTemp >= TEMP_MIN && liveTemp <= TEMP_MAX;
  const live = showLive
    ? project([liveTemp as number, interpolate(draft, liveTemp as number)])
    : null;

  const gridTemps = [40, 60, 80, 100].filter((t) => t <= TEMP_MAX);

  return (
    <div className="flex flex-col gap-1">
      <svg
        ref={svgRef}
        viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
        width="100%"
        height={HEIGHT}
        role="img"
        aria-label="Curve editor"
        className="touch-none select-none"
      >
        {gridTemps.map((t) => {
          const x = PAD + tempToX(t, INNER_W);
          return (
            <line
              key={t}
              x1={x}
              y1={PAD}
              x2={x}
              y2={HEIGHT - PAD}
              stroke="var(--color-separator)"
            />
          );
        })}

        {/* Transparent hit target for click-to-add, behind the line/points. */}
        <rect
          x={0}
          y={0}
          width={WIDTH}
          height={HEIGHT}
          fill="transparent"
          onPointerDown={handleAddPoint}
          className="cursor-copy"
        />

        <path d={path} fill="none" stroke="var(--color-accent)" strokeWidth={2} />

        {draft.map(([temp, pwm], i) => {
          const [x, y] = project([temp, pwm]);
          return (
            <g
              key={i}
              onPointerEnter={() => setHovered(i)}
              onPointerLeave={() => setHovered((h) => (h === i ? null : h))}
            >
              <circle
                cx={x}
                cy={y}
                r={6}
                fill="var(--color-accent)"
                stroke="var(--color-window)"
                strokeWidth={1.5}
                className="cursor-grab active:cursor-grabbing"
                onPointerDown={handlePointerDown(i)}
                onPointerMove={handlePointerMove(i)}
                onPointerUp={handlePointerUp}
              />
              {hovered === i && dragging === null && draft.length > 2 && (
                <text
                  x={x + 9}
                  y={y - 9}
                  fontSize={11}
                  fill="var(--color-error)"
                  className="cursor-pointer select-none"
                  onPointerDown={(e) => {
                    e.stopPropagation();
                    handleRemovePoint(i);
                  }}
                >
                  ×
                </text>
              )}
            </g>
          );
        })}

        {live && (
          <g>
            <line
              x1={live[0]}
              y1={PAD}
              x2={live[0]}
              y2={HEIGHT - PAD}
              stroke="var(--color-warning)"
              strokeDasharray="3,3"
              opacity={0.6}
            />
            <circle
              cx={live[0]}
              cy={live[1]}
              r={5}
              fill="var(--color-warning)"
              stroke="var(--color-window)"
              strokeWidth={1.5}
            />
          </g>
        )}
      </svg>
      {error && <p className="text-xs text-error">{error}</p>}
    </div>
  );
}
