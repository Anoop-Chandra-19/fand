// TypeScript mirror of fand-proto's Status payload (crates/fand-proto).
// PWM values are raw 0-255, exactly what the wire carries.

export interface ChannelStatus {
  rpm: number;
  current_pwm: number;
  target_pwm: number;
  /** "curve" or "override" */
  mode: string;
  override_remaining_s?: number;
}

export interface Status {
  temps: Record<string, number>;
  channels: Record<string, ChannelStatus>;
}

export interface Sample {
  /** Epoch milliseconds at which the frame arrived. */
  at: number;
  status: Status;
}

export function dutyPercent(pwm: number): number {
  return Math.round((pwm * 100) / 255);
}

// Mirrors gui/src-tauri/src/curves.rs's CurveEditorPayload.

/** A `(temp_c, pwm)` point, sorted by temp, linear interpolation between. */
export type CurvePoint = [number, number];

/** A curve owns its temperature source (graph), combines other curves'
 * outputs (mix), or is a constant (flat). */
export type CurveInfo =
  | { kind: "graph"; sensor: string; points: CurvePoint[] }
  | { kind: "mix"; function: string; members: string[] }
  | { kind: "flat"; pwm: number };

export interface CurveEditorPayload {
  curves: Record<string, CurveInfo>;
  /** channel name → the curve it binds. */
  channels: Record<string, string>;
  /** Already-configured sensor names, for graph-curve sensor pickers. */
  sensors: string[];
}

// Mirrors gui/src-tauri/src/settings.rs's ChannelSettings.
export interface ChannelSettings {
  min_pwm: number;
  smoothing_seconds: number;
}

export type ChannelSettingsPayload = Record<string, ChannelSettings>;
