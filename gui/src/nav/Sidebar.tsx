import type { ComponentType } from "react";

export type Page = "overview" | "curves";

interface Props {
  page: Page;
  onChange: (page: Page) => void;
}

// Symbolic-style icons (16x16, currentColor stroke) matching each page's
// actual content, not a generic icon set: Overview mirrors the dashboard's
// ascending duty bars, Curves mirrors the curve editor's own polyline.
function OverviewIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <rect x="2" y="9" width="3" height="5" rx="0.5" fill="currentColor" />
      <rect x="6.5" y="6" width="3" height="8" rx="0.5" fill="currentColor" />
      <rect x="11" y="2" width="3" height="12" rx="0.5" fill="currentColor" />
    </svg>
  );
}

function CurvesIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" aria-hidden="true">
      <path
        d="M2 12 L6 11 L10 5 L14 3"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="2" cy="12" r="1.3" fill="currentColor" />
      <circle cx="10" cy="5" r="1.3" fill="currentColor" />
      <circle cx="14" cy="3" r="1.3" fill="currentColor" />
    </svg>
  );
}

const ITEMS: { page: Page; label: string; icon: ComponentType }[] = [
  { page: "overview", label: "Overview", icon: OverviewIcon },
  { page: "curves", label: "Curves", icon: CurvesIcon },
];

export function Sidebar({ page, onChange }: Props) {
  return (
    <nav className="flex w-52 shrink-0 flex-col gap-0.5 border-r border-separator px-3 pt-4">
      {ITEMS.map(({ page: p, label, icon: Icon }) => {
        const active = p === page;
        return (
          <button
            key={p}
            type="button"
            onClick={() => onChange(p)}
            aria-current={active ? "page" : undefined}
            className={
              "flex h-9 items-center gap-2.5 rounded-lg px-3 text-[14px] transition-colors " +
              (active
                ? "bg-accent/15 text-accent"
                : "text-dim hover:bg-white/5 hover:text-ink")
            }
          >
            <Icon />
            {label}
          </button>
        );
      })}
    </nav>
  );
}
