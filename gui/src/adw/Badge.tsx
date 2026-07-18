import type { ReactNode } from "react";

type Tone = "neutral" | "accent" | "warning" | "success" | "error";

const TONES: Record<Tone, string> = {
  neutral: "bg-white/10 text-dim",
  accent: "bg-accent-badge text-accent",
  warning: "bg-warning-bg/22 text-warning",
  success: "bg-[#26a269]/22 text-success",
  error: "bg-error-bg/22 text-error",
};

/**
 * Small pill status label. Neutral (white-alpha) by default; colored tones
 * carry their meaning through a low-alpha background + standalone color.
 */
export function Badge({
  tone = "neutral",
  className = "",
  children,
}: {
  tone?: Tone;
  className?: string;
  children: ReactNode;
}) {
  return (
    <span
      className={`inline-flex items-center gap-[5px] whitespace-nowrap rounded-full px-[10px] py-px text-[0.82rem] font-bold ${TONES[tone]} ${className}`}
    >
      {children}
    </span>
  );
}
