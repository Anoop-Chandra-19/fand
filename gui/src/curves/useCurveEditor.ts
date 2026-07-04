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

  /** Replaces an existing graph curve's points. */
  const setCurvePoints = useCallback(
    (name: string, points: CurvePoint[]) => runWrite("set_curve_points", { name, points }),
    [runWrite],
  );

  /** Creates a new graph curve bound to `sensor`. */
  const createGraphCurve = useCallback(
    (name: string, sensor: string, points: CurvePoint[]) =>
      runWrite("create_graph_curve", { name, sensor, points }),
    [runWrite],
  );

  /** Rebinds which sensor drives a graph curve. */
  const setGraphSensor = useCallback(
    (name: string, sensor: string) => runWrite("set_graph_sensor", { name, sensor }),
    [runWrite],
  );

  const addMixMember = useCallback(
    (name: string, member: string) => runWrite("add_mix_member", { name, member }),
    [runWrite],
  );

  const removeMixMember = useCallback(
    (name: string, member: string) => runWrite("remove_mix_member", { name, member }),
    [runWrite],
  );

  /** Rebinds which curve drives a channel. */
  const setChannelCurve = useCallback(
    (channel: string, curve: string) => runWrite("set_channel_curve", { channel, curve }),
    [runWrite],
  );

  const deleteCurve = useCallback((name: string) => runWrite("delete_curve", { name }), [runWrite]);

  return {
    data,
    error,
    setCurvePoints,
    createGraphCurve,
    setGraphSensor,
    addMixMember,
    removeMixMember,
    setChannelCurve,
    deleteCurve,
  };
}
