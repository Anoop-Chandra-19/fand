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

export interface CurveRef {
  sensor: string;
  curve: string;
}

export interface ChannelCurveRefs {
  /** One entry for a `single` policy, one per input for `mix`. */
  refs: CurveRef[];
  /** Distinguishes the two shapes even when refs.length === 1 for both. */
  is_mix: boolean;
}

export interface CurveEditorPayload {
  curves: Record<string, CurvePoint[]>;
  channels: Record<string, ChannelCurveRefs>;
  /** Already-configured sensor names, for the "add mix input" picker. */
  sensors: string[];
}
