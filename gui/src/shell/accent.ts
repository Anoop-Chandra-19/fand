/**
 * Runtime accent switching (Preferences → Appearance). Rewrites the three
 * accent tokens from the GNOME hue table in index.css; everything styled
 * with accent utilities follows. The chart series and the warning marker
 * keep their fixed colors on purpose.
 */

export const ACCENTS = ["blue", "teal", "green", "orange", "purple"] as const;
export type Accent = (typeof ACCENTS)[number];

const STORAGE_KEY = "fand-accent";

export function loadAccent(): Accent {
  const saved = localStorage.getItem(STORAGE_KEY);
  return (ACCENTS as readonly string[]).includes(saved ?? "") ? (saved as Accent) : "blue";
}

export function applyAccent(accent: Accent) {
  const root = document.documentElement;
  root.style.setProperty("--color-accent", `var(--accent-${accent})`);
  root.style.setProperty("--color-accent-bg", `var(--accent-${accent}-bg)`);
  const bg = getComputedStyle(root).getPropertyValue(`--accent-${accent}-bg`).trim();
  const hex = bg.replace("#", "");
  if (hex.length >= 6) {
    const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16));
    root.style.setProperty("--color-accent-badge", `rgb(${r} ${g} ${b} / 22%)`);
  }
  localStorage.setItem(STORAGE_KEY, accent);
}
