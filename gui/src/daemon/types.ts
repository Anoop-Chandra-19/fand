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
  /** Bumped by the daemon on every successful config apply; compared
   * against the config payloads' generation to spot stale copies. */
  config_generation: number;
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
 * outputs (mix), is a constant (flat), or latches between two duties
 * (trigger). */
export type CurveInfo =
  | {
      kind: "graph";
      sensor: string;
      points: CurvePoint[];
      hysteresis_up: number;
      hysteresis_down: number;
      response_seconds: number;
    }
  | { kind: "mix"; function: string; members: string[] }
  | { kind: "flat"; pwm: number }
  | {
      kind: "trigger";
      sensor: string;
      idle_temp: number;
      idle_pwm: number;
      load_temp: number;
      load_pwm: number;
      response_seconds: number;
    };

export interface CurveEditorPayload {
  curves: Record<string, CurveInfo>;
  /** channel name → the curve it binds. */
  channels: Record<string, string>;
  /** Already-configured sensor names, for graph-curve sensor pickers. */
  sensors: string[];
  /** The daemon config generation this payload was built from. */
  config_generation: number;
}

// Mirrors gui/src-tauri/src/settings.rs's ChannelSettings.
export interface ChannelSettings {
  min_pwm: number;
  smoothing_seconds: number;
  offset_pwm: number;
}

export interface ChannelSettingsPayload {
  channels: Record<string, ChannelSettings>;
  /** The daemon config generation this payload was built from. */
  config_generation: number;
}
