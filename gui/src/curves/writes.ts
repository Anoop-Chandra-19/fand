import { invoke } from "@tauri-apps/api/core";
import type { CurvePoint, WriteResult } from "../daemon/types";

/**
 * The curve write commands. Fire-and-report: each resolves to a
 * WriteResult — `error` when the write failed (or its outcome is
 * unknown), `warning` when it succeeded with a caveat the user must see
 * (rides the invoke result so each operation produces exactly one
 * toast) — and never throws. The applied config itself comes back
 * through the backend's "config" event; no caller holds config state.
 */
async function runWrite(command: string, args: Record<string, unknown>): Promise<WriteResult> {
  try {
    const warning = await invoke<string | null>(command, args);
    return { error: null, warning: warning ?? null };
  } catch (e) {
    return { error: String(e), warning: null };
  }
}

export const curveWrites = {
  /** Replaces an existing graph curve's points. */
  setCurvePoints: (name: string, points: CurvePoint[]) =>
    runWrite("set_curve_points", { name, points }),

  /** Creates a new graph curve bound to `sensor`. */
  createGraphCurve: (name: string, sensor: string, points: CurvePoint[]) =>
    runWrite("create_graph_curve", { name, sensor, points }),

  /** Rebinds which sensor drives a graph curve. */
  setGraphSensor: (name: string, sensor: string) => runWrite("set_graph_sensor", { name, sensor }),

  /** Applies a full graph-curve edit as one batch (one SetConfig). */
  applyGraphCurve: (
    name: string,
    sensor: string,
    points: CurvePoint[],
    hysteresisUp: number,
    hysteresisDown: number,
    responseSeconds: number,
  ) =>
    runWrite("apply_graph_curve", {
      name,
      sensor,
      points,
      hysteresisUp,
      hysteresisDown,
      responseSeconds,
    }),

  /** Creates a new flat curve holding a constant pwm. */
  createFlatCurve: (name: string, pwm: number) => runWrite("create_flat_curve", { name, pwm }),

  /** Changes an existing flat curve's constant pwm. */
  setFlatPwm: (name: string, pwm: number) => runWrite("set_flat_pwm", { name, pwm }),

  /** Creates a new mix curve combining `members` with `function`. */
  createMixCurve: (name: string, fn: string, members: string[]) =>
    runWrite("create_mix_curve", { name, function: fn, members }),

  /** Changes an existing mix curve's combining function. */
  setMixFunction: (name: string, fn: string) => runWrite("set_mix_function", { name, function: fn }),

  /** Creates a new trigger curve (the daemon enforces the pwm1 ban). */
  createTriggerCurve: (
    name: string,
    sensor: string,
    idleTemp: number,
    idlePwm: number,
    loadTemp: number,
    loadPwm: number,
    responseSeconds: number,
  ) =>
    runWrite("create_trigger_curve", {
      name,
      sensor,
      idleTemp,
      idlePwm,
      loadTemp,
      loadPwm,
      responseSeconds,
    }),

  /** Applies a full trigger-curve edit as one batch. */
  applyTriggerCurve: (
    name: string,
    sensor: string,
    idleTemp: number,
    idlePwm: number,
    loadTemp: number,
    loadPwm: number,
    responseSeconds: number,
  ) =>
    runWrite("apply_trigger_curve", {
      name,
      sensor,
      idleTemp,
      idlePwm,
      loadTemp,
      loadPwm,
      responseSeconds,
    }),

  addMixMember: (name: string, member: string) => runWrite("add_mix_member", { name, member }),

  removeMixMember: (name: string, member: string) =>
    runWrite("remove_mix_member", { name, member }),

  /** Rebinds which curve drives a channel. */
  setChannelCurve: (channel: string, curve: string) =>
    runWrite("set_channel_curve", { channel, curve }),

  deleteCurve: (name: string) => runWrite("delete_curve", { name }),
};
