import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { CurveEditorPayload, CurvePoint } from "../daemon/types";

/** Fetches the curve editor's data on mount and exposes the write path. */
export function useCurveEditor() {
  const [data, setData] = useState<CurveEditorPayload | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    return invoke<CurveEditorPayload>("get_curve_editor_data").then((payload) => {
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
        const payload = await invoke<CurveEditorPayload>(command, args);
        setData(payload);
        return null;
      } catch (e) {
        return String(e);
      }
    },
    [],
  );

  /** Edits an existing curve, or creates one with these points if `name` is new. */
  const setCurvePoints = useCallback(
    (name: string, points: CurvePoint[]) => runWrite("set_curve_points", { name, points }),
    [runWrite],
  );

  const deleteCurve = useCallback((name: string) => runWrite("delete_curve", { name }), [runWrite]);

  const setChannelCurve = useCallback(
    (channel: string, sensor: string, curve: string) =>
      runWrite("set_channel_curve", { channel, sensor, curve }),
    [runWrite],
  );

  const addMixInput = useCallback(
    (channel: string, sensor: string, curve: string) =>
      runWrite("add_mix_input", { channel, sensor, curve }),
    [runWrite],
  );

  const removeMixInput = useCallback(
    (channel: string, sensor: string) => runWrite("remove_mix_input", { channel, sensor }),
    [runWrite],
  );

  return {
    data,
    error,
    setCurvePoints,
    deleteCurve,
    setChannelCurve,
    addMixInput,
    removeMixInput,
  };
}
