import type { CurvePoint } from "../../api/daemonTypes";
import { invokeWrite } from "../../api/invokeWrite";

/**
 * The curve write commands. Fire-and-report: each resolves to a
 * WriteResult — `error` when the write failed (or its outcome is
 * unknown), `warning` when it succeeded with a caveat the user must see
 * (rides the invoke result so each operation produces exactly one
 * toast) — and never throws. The applied config itself comes back
 * through the backend's "config" event; no caller holds config state.
 */
export const curveCommands = {
  /** Replaces an existing graph curve's points. */
  setCurvePoints: (name: string, points: CurvePoint[]) =>
    invokeWrite("set_curve_points", { name, points }),

  /** Creates a new graph curve bound to `sensor`. */
  createGraphCurve: (name: string, sensor: string, points: CurvePoint[]) =>
    invokeWrite("create_graph_curve", { name, sensor, points }),

  /** Rebinds which sensor drives a graph curve. */
  setGraphSensor: (name: string, sensor: string) =>
    invokeWrite("set_graph_sensor", { name, sensor }),

  /** Applies a full graph-curve edit as one batch (one SetConfig). */
  applyGraphCurve: (
    name: string,
    sensor: string,
    points: CurvePoint[],
    hysteresisUp: number,
    hysteresisDown: number,
    responseSeconds: number,
  ) =>
    invokeWrite("apply_graph_curve", {
      name,
      sensor,
      points,
      hysteresisUp,
      hysteresisDown,
      responseSeconds,
    }),

  /** Creates a new flat curve holding a constant pwm. */
  createFlatCurve: (name: string, pwm: number) =>
    invokeWrite("create_flat_curve", { name, pwm }),

  /** Changes an existing flat curve's constant pwm. */
  setFlatPwm: (name: string, pwm: number) => invokeWrite("set_flat_pwm", { name, pwm }),

  /** Creates a new mix curve combining `members` with `function`. */
  createMixCurve: (name: string, fn: string, members: string[]) =>
    invokeWrite("create_mix_curve", { name, function: fn, members }),

  /** Changes an existing mix curve's combining function. */
  setMixFunction: (name: string, fn: string) =>
    invokeWrite("set_mix_function", { name, function: fn }),

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
    invokeWrite("create_trigger_curve", {
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
    invokeWrite("apply_trigger_curve", {
      name,
      sensor,
      idleTemp,
      idlePwm,
      loadTemp,
      loadPwm,
      responseSeconds,
    }),

  addMixMember: (name: string, member: string) =>
    invokeWrite("add_mix_member", { name, member }),

  removeMixMember: (name: string, member: string) =>
    invokeWrite("remove_mix_member", { name, member }),

  deleteCurve: (name: string) => invokeWrite("delete_curve", { name }),
};
