import { useEffect, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { CloseIcon, MenuIcon } from "../adw/icons";

/**
 * Native GNOME CSD: the app draws its own headerbar (the window runs with
 * server decorations disabled). Flat over the window color with a shade
 * hairline; `data-tauri-drag-region` gives drag + double-click-maximize.
 * GNOME's default button layout is close-only.
 */
export function HeaderBar({
  title,
  subtitle,
  menuItems,
}: {
  title: string;
  subtitle?: string;
  menuItems: { label: string; onClick: () => void }[];
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [menuOpen]);
  return (
    <header
      data-tauri-drag-region
      className="relative z-2 flex h-[47px] shrink-0 select-none items-center justify-between px-2 shadow-[0_1px_0_var(--headerbar-shade)]"
    >
      <div className="z-1 min-w-[40px]" />
      <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-px">
        <span className="text-[1rem] font-bold leading-[1.1]">{title}</span>
        {subtitle && <span className="text-[0.82rem] leading-[1.1] text-dim">{subtitle}</span>}
      </div>
      <div className="relative z-1 flex items-center gap-1">
        <HeaderButton
          label="Primary menu"
          expanded={menuOpen}
          onClick={() => setMenuOpen((o) => !o)}
        >
          <MenuIcon />
        </HeaderButton>
        <HeaderButton label="Close" round onClick={() => void getCurrentWindow().close()}>
          <CloseIcon />
        </HeaderButton>
        {menuOpen && (
          <div
            role="menu"
            onMouseLeave={() => setMenuOpen(false)}
            className="absolute right-0 top-10 z-60 min-w-[214px] rounded-popover bg-popover p-[6px] shadow-popover"
          >
            {menuItems.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenuOpen(false);
                  item.onClick();
                }}
                className="block w-full cursor-pointer rounded-row px-[10px] py-[7px] text-left text-ink hover:bg-[var(--flat-hover-fill)]"
              >
                {item.label}
              </button>
            ))}
          </div>
        )}
      </div>
    </header>
  );
}

function HeaderButton({
  label,
  round = false,
  expanded,
  onClick,
  children,
}: {
  label: string;
  round?: boolean;
  /** Set when the button controls a popover menu. */
  expanded?: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-haspopup={expanded === undefined ? undefined : "menu"}
      aria-expanded={expanded}
      onClick={onClick}
      className={`flex h-[34px] w-[34px] cursor-pointer items-center justify-center text-ink transition-colors duration-200 ${
        round ? "rounded-full bg-white/9 hover:bg-white/13" : "rounded-button hover:bg-[var(--flat-hover-fill)]"
      }`}
    >
      {children}
    </button>
  );
}
