import { listen } from "@tauri-apps/api/event";
import { useEffect, useState } from "react";
import type { Status } from "./types";

export interface Sample {
  /** Epoch milliseconds at which the frame arrived. */
  at: number;
  status: Status;
}

/**
 * Live view of the daemon: latest frame, a rolling history for the charts,
 * and whether the socket is currently delivering. The backend pushes one
 * "status" event per daemon tick (~2 s) and "daemon-down" when the
 * connection drops.
 */
export function useDaemonStatus(maxSamples = 450) {
  // null = no frame yet since launch ("connecting"), false = daemon-down.
  const [connected, setConnected] = useState<boolean | null>(null);
  const [latest, setLatest] = useState<Status | null>(null);
  const [history, setHistory] = useState<Sample[]>([]);

  useEffect(() => {
    const unlistenStatus = listen<Status>("status", (event) => {
      setConnected(true);
      setLatest(event.payload);
      setHistory((prev) => {
        const next = [...prev, { at: Date.now(), status: event.payload }];
        return next.length > maxSamples ? next.slice(-maxSamples) : next;
      });
    });
    const unlistenDown = listen("daemon-down", () => setConnected(false));
    return () => {
      unlistenStatus.then((f) => f());
      unlistenDown.then((f) => f());
    };
  }, [maxSamples]);

  return { connected, latest, history };
}
