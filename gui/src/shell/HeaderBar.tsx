import { useEffect, useRef, useState, type ReactNode, type Ref } from "react";
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
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  // role="menu" is a keyboard contract, not decoration: focus moves into
  // the menu on open, arrows cycle it (Home/End jump), and every close
  // path hands focus back to the button — focus lives inside the menu, so
  // closing without restoring would drop it on <body>.
  useEffect(() => {
    if (!menuOpen) return;
    const items = () =>
      Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
    items()[0]?.focus();
    const onPointerDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!menuRef.current?.contains(t) && !menuButtonRef.current?.contains(t)) {
        setMenuOpen(false);
        // Focus was inside the menu; park it on the button. A focusable
        // click target then takes it anyway (its focus happens after
        // pointerdown), but a non-focusable one no longer drops focus
        // onto <body>.
        menuButtonRef.current?.focus();
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setMenuOpen(false);
        menuButtonRef.current?.focus();
        return;
      }
      if (e.key === "Tab") {
        // Refocus the button first (no preventDefault): the browser then
        // tabs onward from it, per the normal tab sequence.
        setMenuOpen(false);
        menuButtonRef.current?.focus();
        return;
      }
      const list = items();
      if (list.length === 0) return;
      const at = list.indexOf(document.activeElement as HTMLButtonElement);
      let to = -1;
      if (e.key === "ArrowDown") to = (at + 1) % list.length;
      else if (e.key === "ArrowUp") to = at <= 0 ? list.length - 1 : at - 1;
      else if (e.key === "Home") to = 0;
      else if (e.key === "End") to = list.length - 1;
      if (to >= 0) {
        e.preventDefault();
        list[to].focus();
      }
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("pointerdown", onPointerDown);
    return () => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("pointerdown", onPointerDown);
    };
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
          buttonRef={menuButtonRef}
          onClick={() => setMenuOpen((o) => !o)}
        >
          <MenuIcon />
        </HeaderButton>
        <HeaderButton label="Close" round onClick={() => void getCurrentWindow().close()}>
          <CloseIcon />
        </HeaderButton>
        {menuOpen && (
          <div
            ref={menuRef}
            role="menu"
            aria-label="Primary menu"
            className="absolute right-0 top-10 z-60 min-w-[214px] rounded-popover bg-popover p-[6px] shadow-popover"
          >
            {menuItems.map((item) => (
              <button
                key={item.label}
                type="button"
                role="menuitem"
                tabIndex={-1}
                onClick={() => {
                  // Refocus the button before acting: the item is about to
                  // unmount, and a dialog the action opens snapshots the
                  // focused element to restore on close.
                  menuButtonRef.current?.focus();
                  setMenuOpen(false);
                  item.onClick();
                }}
                className="block w-full cursor-pointer rounded-row px-[10px] py-[7px] text-left text-ink hover:bg-[var(--flat-hover-fill)] focus-visible:bg-[var(--flat-hover-fill)] focus-visible:outline-none"
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
  buttonRef,
  onClick,
  children,
}: {
  label: string;
  round?: boolean;
  /** Set when the button controls a popover menu. */
  expanded?: boolean;
  buttonRef?: Ref<HTMLButtonElement>;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      ref={buttonRef}
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
