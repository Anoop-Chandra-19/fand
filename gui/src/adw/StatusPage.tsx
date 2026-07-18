import type { ReactNode } from "react";

/**
 * AdwStatusPage — a big centered icon + title + description for a
 * full-page empty or error state.
 */
export function StatusPage({
  icon,
  title,
  description,
  children,
}: {
  icon?: ReactNode;
  title: string;
  description?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex flex-col items-center justify-center gap-3 px-6 py-12 text-center">
      {icon && (
        <div className="mb-[6px] opacity-55" aria-hidden="true">
          {icon}
        </div>
      )}
      <h2 className="m-0 text-[1.36rem] font-extrabold">{title}</h2>
      {description && (
        <p className="m-0 max-w-[360px] leading-[1.4] text-dim">{description}</p>
      )}
      {children && <div className="mt-[6px]">{children}</div>}
    </div>
  );
}
