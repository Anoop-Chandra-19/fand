import { invoke } from "@tauri-apps/api/core";
import type { WriteResult } from "../daemon/types";

/**
 * Channel-settings write commands — same contract as `curveWrites`: a
 * WriteResult with `error` on failure or `warning` for
 * applied-with-caveat, never a throw. The applied config arrives via
 * the backend's "config" event.
 */
async function runWrite(command: string, args: Record<string, unknown>): Promise<WriteResult> {
  try {
    const warning = await invoke<string | null>(command, args);
    return { error: null, warning: warning ?? null };
  } catch (e) {
    return { error: String(e), warning: null };
  }
}

export const setMinPwm = (channel: string, minPwm: number) =>
  runWrite("set_min_pwm", { channel, minPwm });

export const setSmoothingSeconds = (channel: string, seconds: number) =>
  runWrite("set_smoothing_seconds", { channel, seconds });

export const setOffsetPwm = (channel: string, offset: number) =>
  runWrite("set_offset_pwm", { channel, offset });

/** Cancels a manual override; the next status frame reflects the change. */
export const clearOverride = (channel: string) => runWrite("clear_override", { channel });
