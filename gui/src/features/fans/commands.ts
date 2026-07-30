import { invokeWrite } from "../../api/invokeWrite";

export const setMinPwm = (channel: string, minPwm: number) =>
  invokeWrite("set_min_pwm", { channel, minPwm });

export const setSmoothingSeconds = (channel: string, seconds: number) =>
  invokeWrite("set_smoothing_seconds", { channel, seconds });

export const setOffsetPwm = (channel: string, offset: number) =>
  invokeWrite("set_offset_pwm", { channel, offset });

export const setChannelCurve = (channel: string, curve: string) =>
  invokeWrite("set_channel_curve", { channel, curve });

/** Cancels a manual override; the next status frame reflects the change. */
export const clearOverride = (channel: string) => invokeWrite("clear_override", { channel });
