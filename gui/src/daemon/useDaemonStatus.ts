import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { ConfigPayload, Sample, Status, StatusEvent } from "./types";

/**
 * Live view of the daemon — the frontend's only data source. The backend
 * owns the config copy and pushes everything: one "status" event per
 * daemon tick (frame + config), a "config" event immediately after each
 * write it applied, and "daemon-down" (repeated every retry) while the
 * socket is dead. This side just keeps the last thing it heard; there is
 * deliberately no fetching, caching or reconciling here.
 */
export function useDaemonStatus(windowMs = 15 * 60_000) {
  // null = no frame yet since launch ("connecting"), false = daemon-down.
  const [connected, setConnected] = useState<boolean | null>(null);
  const [latest, setLatest] = useState<Status | null>(null);
  const [config, setConfig] = useState<ConfigPayload | null>(null);
  const [history, setHistory] = useState<Sample[]>([]);

  useEffect(() => {
    const unlistenStatus = listen<StatusEvent>("status", (event) => {
      setConnected(true);
      setLatest(event.payload.status);
      setConfig(event.payload.config);
      setHistory((prev) => {
        // Trim by sample age, not count — the tick interval is the
        // daemon's to choose, and disconnect gaps deliver no frames.
        const cutoff = Date.now() - windowMs;
        const next = prev.filter((s) => s.at >= cutoff);
        next.push({ at: Date.now(), status: event.payload.status });
        return next;
      });
    });
    const unlistenConfig = listen<ConfigPayload>("config", (event) => {
      setConfig(event.payload);
    });
    const unlistenDown = listen("daemon-down", () => {
      setConnected(false);
      // A dead daemon's config is not current config: whatever comes back
      // may have restarted with a different file on disk.
      setConfig(null);
    });
    return () => {
      unlistenStatus.then((f) => f());
      unlistenConfig.then((f) => f());
      unlistenDown.then((f) => f());
    };
  }, [windowMs]);

  return { connected, latest, config, history };
}
