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

export function dutyPercent(pwm: number): number {
  return Math.round((pwm * 100) / 255);
}
