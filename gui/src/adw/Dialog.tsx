import { useEffect, useRef, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";

/**
 * AdwDialog — a floating sheet: dimmed backdrop, 15px radius, deeper
 * shadow, sized to content, never a fullscreen takeover. Clicking the
 * backdrop or pressing Escape closes it. Focus moves into the sheet on
 * open, Tab cycles inside it, and the opener gets focus back on close.
 */
export function Dialog({
  width,
  label,
  onClose,
  children,
}: {
  width: number;
  /** Accessible name announced when focus enters the dialog. */
  label: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const sheetRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    sheetRef.current?.focus();
    return () => opener?.focus();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const trapTab = (e: ReactKeyboardEvent) => {
    if (e.key !== "Tab") return;
    const sheet = sheetRef.current;
    if (!sheet) return;
    const focusables = sheet.querySelectorAll<HTMLElement>(
      'button:not([disabled]), select:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
    );
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === sheet)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      onPointerDown={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-[rgb(0_0_6/45%)] p-5"
    >
      <div
        ref={sheetRef}
        role="dialog"
        aria-modal="true"
        aria-label={label}
        tabIndex={-1}
        onKeyDown={trapTab}
        onPointerDown={(e) => e.stopPropagation()}
        style={{ width }}
        className="flex max-h-[92vh] max-w-[94vw] flex-col overflow-hidden rounded-dialog bg-dialog shadow-dialog outline-none"
      >
        {children}
      </div>
    </div>
  );
}

/** The 34px round × button used by close-only dialog headers. */
export function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <button
      type="button"
      onClick={onClose}
      aria-label="Close"
      className="flex h-[34px] w-[34px] shrink-0 cursor-pointer items-center justify-center rounded-full bg-white/9 text-[16px] text-ink"
    >
      ×
    </button>
  );
}

/** Dialog header: left / centered title / right slots over a hairline. */
export function DialogHeader({
  left,
  right,
  title,
  subtitle,
  mono = false,
}: {
  left?: ReactNode;
  right?: ReactNode;
  title: ReactNode;
  subtitle?: ReactNode;
  mono?: boolean;
}) {
  return (
    <div className="flex shrink-0 items-center justify-between gap-2 border-b border-separator px-3 py-[10px]">
      {left}
      <div className="text-center leading-[1.1]">
        <div className={`font-bold ${mono ? "font-mono" : ""}`}>{title}</div>
        {subtitle && <div className="text-[0.82rem] text-dim">{subtitle}</div>}
      </div>
      {right}
    </div>
  );
}
