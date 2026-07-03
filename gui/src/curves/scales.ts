// Pure coordinate math for the curve editor SVG — no rendering, no state.
import type { CurvePoint } from "../daemon/types";

export const TEMP_MIN = 20;
export const TEMP_MAX = 100;
export const PWM_MAX = 255;

export function tempToX(temp: number, width: number): number {
  return ((temp - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)) * width;
}

export function pwmToY(pwm: number, height: number): number {
  return height - (pwm / PWM_MAX) * height;
}

export function xToTemp(x: number, width: number): number {
  return TEMP_MIN + (x / width) * (TEMP_MAX - TEMP_MIN);
}

export function yToPwm(y: number, height: number): number {
  return ((height - y) / height) * PWM_MAX;
}

/** Mirrors fand-core's `Curve::at` — clamped linear interpolation. */
export function interpolate(points: CurvePoint[], temp: number): number {
  const first = points[0];
  const last = points[points.length - 1];
  if (temp <= first[0]) return first[1];
  if (temp >= last[0]) return last[1];
  for (let i = 1; i < points.length; i++) {
    const [t0, p0] = points[i - 1];
    const [t1, p1] = points[i];
    if (temp <= t1) {
      return p0 + ((temp - t0) / (t1 - t0)) * (p1 - p0);
    }
  }
  return last[1];
}
