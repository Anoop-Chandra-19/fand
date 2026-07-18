import type { ReactNode } from "react";

/**
 * libadwaita `.card` — the white-8% surface with 12px radius and a subtle
 * shade shadow. No border, no colored edge. `activatable` adds a hover
 * lift for clickable cards.
 */
export function Card({
  activatable = false,
  onClick,
  className = "",
  children,
}: {
  activatable?: boolean;
  onClick?: () => void;
  className?: string;
  children: ReactNode;
}) {
  const surface = "rounded-card bg-card p-[16px] shadow-card transition-colors duration-200 ease-out";
  // Activatable cards are real buttons so the keyboard can open them.
  if (activatable) {
    return (
      <button
        type="button"
        onClick={onClick}
        className={`${surface} w-full cursor-pointer text-left hover:bg-white/11 ${className}`}
      >
        {children}
      </button>
    );
  }
  return <div className={`${surface} ${className}`}>{children}</div>;
}
