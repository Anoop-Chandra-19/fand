/** App-level preferences persisted in localStorage (GUI-only knobs). */

const CHART_KEY = "fand-chart-minutes";
export const CHART_MINUTES_DEFAULT = 15;

export function loadChartMinutes(): number {
  const saved = Number(localStorage.getItem(CHART_KEY));
  return Number.isFinite(saved) && saved >= 5 && saved <= 30 ? saved : CHART_MINUTES_DEFAULT;
}

export function saveChartMinutes(minutes: number) {
  localStorage.setItem(CHART_KEY, String(minutes));
}

/** The rolling-history window in milliseconds. */
export function chartWindowMs(minutes: number): number {
  return minutes * 60_000;
}
