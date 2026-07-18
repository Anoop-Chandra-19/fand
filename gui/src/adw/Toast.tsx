import type { ReactNode } from "react";

/**
 * AdwToast — a floating dark pill, bottom-center, for a transient
 * confirmation ("Curve cpu_rad applied"). The overlay container pins it;
 * pointer events pass through everywhere but the pill itself.
 */
export function ToastOverlay({ toast }: { toast: ReactNode | null }) {
  if (!toast) return null;
  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-[22px] z-80 flex justify-center">
      <div
        role="status"
        className="pointer-events-auto inline-flex items-center gap-3 rounded-full bg-popover py-2 pl-4 pr-4 shadow-popover"
      >
        {toast}
      </div>
    </div>
  );
}
