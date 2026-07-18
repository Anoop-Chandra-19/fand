import type { CurveInfo, CurvePoint } from "./types";

/** Linear interpolation of a graph curve at temp `t` (endpoint hold). */
export function interpolate(points: CurvePoint[], t: number): number {
  if (points.length === 0) return 0;
  if (t <= points[0][0]) return points[0][1];
  if (t >= points[points.length - 1][0]) return points[points.length - 1][1];
  for (let i = 1; i < points.length; i++) {
    const [t0, p0] = points[i - 1];
    const [t1, p1] = points[i];
    if (t <= t1) return p0 + ((p1 - p0) * (t - t0)) / (t1 - t0);
  }
  return points[points.length - 1][1];
}

/**
 * Evaluates a curve tree to a pwm 0–255 for the "duty now" readouts.
 * Returns null when the result isn't knowable client-side: a missing
 * sensor reading, or any trigger in the tree — a trigger's latched state
 * lives in the daemon, and guessing it here could show a wrong duty.
 *
 * Per the mix safety rule, members are evaluated at their own sensors and
 * their *outputs* combined — never one curve fed a combined temperature.
 */
export function evalCurve(
  curves: Record<string, CurveInfo>,
  name: string,
  temps: Record<string, number>,
): number | null {
  const c = curves[name];
  if (!c) return null;
  if (c.kind === "graph") {
    const t = temps[c.sensor];
    return t === undefined ? null : interpolate(c.points, t);
  }
  if (c.kind === "flat") return c.pwm;
  if (c.kind === "mix") {
    const outs: number[] = [];
    for (const member of c.members) {
      const out = evalCurve(curves, member, temps);
      if (out === null) return null;
      outs.push(out);
    }
    if (outs.length === 0) return null;
    if (c.function === "min") return Math.min(...outs);
    if (c.function === "average") return outs.reduce((a, b) => a + b, 0) / outs.length;
    return Math.max(...outs);
  }
  return null; // trigger — latched daemon-side
}
