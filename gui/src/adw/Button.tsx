import type { ReactNode } from "react";

type Variant = "regular" | "flat" | "suggested" | "destructive";

const FILLS: Record<Variant, string> = {
  regular: "bg-white/9 hover:bg-white/13",
  flat: "hover:bg-[var(--flat-hover-fill)] active:bg-[var(--flat-active-fill)]",
  suggested: "bg-accent-bg text-white hover:brightness-[1.08]",
  destructive: "bg-destructive-bg text-white hover:brightness-110",
};

/** libadwaita button. `circular` makes the 34px round icon button. */
export function Button({
  variant = "regular",
  circular = false,
  disabled = false,
  onClick,
  className = "",
  children,
}: {
  variant?: Variant;
  circular?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  className?: string;
  children: ReactNode;
}) {
  const shape = circular
    ? "h-[34px] w-[34px] rounded-full"
    : "rounded-button px-[10px] py-[5px] min-h-[24px]";
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={`inline-flex select-none items-center justify-center gap-[6px] font-bold leading-tight transition-[background-color,filter] duration-200 ease-out disabled:pointer-events-none disabled:opacity-45 ${shape} ${FILLS[variant]} ${className}`}
    >
      {children}
    </button>
  );
}
