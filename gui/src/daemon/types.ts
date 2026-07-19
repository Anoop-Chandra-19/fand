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
  /** Bumped by the daemon on every successful config apply. The backend
   * uses it to keep its config cache fresh; here it is informational. */
  config_generation: number;
  /** Random per-daemon-process token; generations are only ordered within
   * one instance. Consumed by the backend, informational here. */
  instance: number;
}

export interface Sample {
  /** Epoch milliseconds at which the frame arrived. */
  at: number;
  status: Status;
}

export function dutyPercent(pwm: number): number {
  return Math.round((pwm * 100) / 255);
}

// Mirrors gui/src-tauri/src/state.rs's payload types. The backend owns
// the only config copy and pushes it with events; nothing on this side
// fetches or caches config.

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

export interface ChannelSettings {
  min_pwm: number;
  smoothing_seconds: number;
  offset_pwm: number;
}

/** Everything about the daemon's config, in one coherent piece. */
export interface ConfigPayload {
  curves: Record<string, CurveInfo>;
  /** channel name → the curve it binds. */
  channels: Record<string, string>;
  /** Already-configured sensor names, for graph-curve sensor pickers. */
  sensors: string[];
  channel_settings: Record<string, ChannelSettings>;
  /** The daemon config generation this payload was built from. */
  config_generation: number;
  /** The daemon lifetime the generation belongs to (see Status.instance). */
  instance: number;
  /** The daemon's control-loop interval in seconds. */
  tick_seconds: number;
}

/** How a write command landed. At most one of the fields is set: `error`
 * means it failed (or its outcome is unknown — the message says so);
 * `warning` means it succeeded with a caveat the user must see. Both
 * null is a clean success. */
export interface WriteResult {
  error: string | null;
  warning: string | null;
}

/** One status frame plus the newest same-instance config that covers it.
 * Usually exactly the config the frame was computed under; right after
 * writes it may transiently run ahead of a queued frame (same instance,
 * higher generation) — never behind, never another daemon lifetime's. */
export interface StatusEvent {
  status: Status;
  /** null when no covering config is known for this frame (the backend's
   * fetch failed; the next frame retries). */
  config: ConfigPayload | null;
}
