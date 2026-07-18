import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { ChannelSettingsPayload } from "../daemon/types";

/** Fetches per-channel settings on mount and exposes the write path. */
export function useChannelSettings() {
  const [data, setData] = useState<ChannelSettingsPayload | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    return invoke<ChannelSettingsPayload>("get_channel_settings").then((payload) => {
      setData(payload);
      return payload;
    });
  }, []);

  useEffect(() => {
    let cancelled = false;
    refresh().catch((e) => {
      if (!cancelled) setError(String(e));
    });
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  /**
   * Every write command shares this contract: returns null on success (and
   * updates `data` from the daemon's post-write state), or an error string
   * on failure — never throws, so callers can revert their own optimistic
   * UI without a try/catch.
   */
  const runWrite = useCallback(
    async (command: string, args: Record<string, unknown>): Promise<string | null> => {
      try {
        const payload = await invoke<ChannelSettingsPayload>(command, args);
        setData(payload);
        return null;
      } catch (e) {
        return String(e);
      }
    },
    [],
  );

  const setMinPwm = useCallback(
    (channel: string, minPwm: number) => runWrite("set_min_pwm", { channel, minPwm }),
    [runWrite],
  );

  const setSmoothingSeconds = useCallback(
    (channel: string, seconds: number) => runWrite("set_smoothing_seconds", { channel, seconds }),
    [runWrite],
  );

  const setOffsetPwm = useCallback(
    (channel: string, offset: number) => runWrite("set_offset_pwm", { channel, offset }),
    [runWrite],
  );

  return {
    data,
    error,
    refresh,
    setMinPwm,
    setSmoothingSeconds,
    setOffsetPwm,
  };
}

/** Cancels a manual override; the next status frame reflects the change. */
export async function clearOverride(channel: string): Promise<string | null> {
  try {
    await invoke("clear_override", { channel });
    return null;
  } catch (e) {
    return String(e);
  }
}
