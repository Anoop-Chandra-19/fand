import type { ReactNode } from "react";
import { WarnIcon } from "./icons";

type Tone = "warning" | "error";

const TONES: Record<Tone, { tint: string; line: string; ink: string }> = {
  warning: {
    tint: "rgb(205 147 9 / 15%)",
    line: "rgb(205 147 9 / 55%)",
    ink: "var(--color-warning)",
  },
  error: {
    tint: "rgb(192 28 40 / 18%)",
    line: "rgb(192 28 40 / 65%)",
    ink: "var(--color-error)",
  },
};

/**
 * AdwBanner — a full-width bar under the headerbar for a persistent state
 * (override active, daemon unreachable): tinted surface, leading status
 * icon, left-aligned message, optional trailing action.
 */
export function Banner({
  tone = "warning",
  action,
  onAction,
  children,
}: {
  tone?: Tone;
  action?: string;
  onAction?: () => void;
  children: ReactNode;
}) {
  const t = TONES[tone];
  return (
    <div
      role="alert"
      style={{ background: t.tint, borderBottom: `1px solid ${t.line}`, color: t.ink }}
    >
      <div className="mx-auto flex max-w-[1080px] items-center gap-[10px] px-5 py-[9px]">
        <WarnIcon />
        <span className="min-w-0 flex-1 font-bold text-ink">{children}</span>
        {action && (
          <button
            type="button"
            onClick={onAction}
            className="shrink-0 cursor-pointer whitespace-nowrap rounded-full border bg-transparent px-[14px] py-[3px] text-[0.82rem] font-bold transition-colors duration-200"
            style={{ borderColor: t.line, color: t.ink }}
          >
            {action}
          </button>
        )}
      </div>
    </div>
  );
}
