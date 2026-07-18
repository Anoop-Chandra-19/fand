import { Children, type ReactNode } from "react";

/**
 * Boxed-list forms — the AdwActionRow family. A `BoxedList` wraps rows on
 * a card surface with hairline separators; the rows put a title (+ dim
 * subtitle) left and a control right, applying instantly.
 */

const CHEVRON =
  "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='16' height='16' viewBox='0 0 16 16'%3E%3Cpath d='M4 6l4 4 4-4' stroke='%23ffffff' stroke-opacity='0.55' stroke-width='1.5' fill='none' stroke-linecap='round' stroke-linejoin='round'/%3E%3C/svg%3E\")";

/** Dark-styled native select, shared by every dropdown in the app. */
export function Select({
  value,
  options,
  disabled,
  mono = false,
  ariaLabel,
  onChange,
  className = "",
}: {
  value: string;
  options: (string | { value: string; label: string })[];
  disabled?: boolean;
  /** Mono/caption variant for inline pickers of config identifiers. */
  mono?: boolean;
  ariaLabel?: string;
  onChange: (value: string) => void;
  className?: string;
}) {
  return (
    <select
      value={value}
      disabled={disabled}
      aria-label={ariaLabel}
      onChange={(e) => onChange(e.target.value)}
      className={`cursor-pointer appearance-none rounded-button border-none bg-white/9 pl-[10px] text-ink ${
        mono ? "numeric py-[5px] pr-[26px] text-[0.82rem]" : "py-[5px] pr-[28px]"
      } ${className}`}
      style={{
        backgroundImage: CHEVRON,
        backgroundRepeat: "no-repeat",
        backgroundPosition: "right 8px center",
      }}
    >
      {options.map((o) => {
        const val = typeof o === "string" ? o : o.value;
        const label = typeof o === "string" ? o : o.label;
        return (
          <option key={val} value={val}>
            {label}
          </option>
        );
      })}
    </select>
  );
}

export function BoxedList({ className = "", children }: { className?: string; children: ReactNode }) {
  const rows = Children.toArray(children).filter(Boolean);
  return (
    <div className={`overflow-hidden rounded-card bg-card shadow-card ${className}`}>
      {rows.map((row, i) => (
        <div key={i} className={i === 0 ? "" : "border-t border-separator"}>
          {row}
        </div>
      ))}
    </div>
  );
}

export function ActionRow({
  title,
  subtitle,
  trailing,
  activatable = false,
  onClick,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  trailing?: ReactNode;
  activatable?: boolean;
  onClick?: () => void;
}) {
  const layout = "flex min-h-[50px] w-full items-center gap-3 px-3 py-2 transition-colors duration-200";
  const content = (
    <>
      <div className="flex min-w-0 flex-1 flex-col gap-px text-left">
        <span>{title}</span>
        {subtitle && <span className="text-[0.82rem] text-dim">{subtitle}</span>}
      </div>
      {trailing && <div className="flex shrink-0 items-center gap-2">{trailing}</div>}
    </>
  );
  // An activatable row is a real button, so it is reachable and
  // triggerable from the keyboard, not just by pointer.
  if (activatable) {
    return (
      <button type="button" onClick={onClick} className={`${layout} cursor-pointer hover:bg-white/5`}>
        {content}
      </button>
    );
  }
  return <div className={layout}>{content}</div>;
}

export function ComboRow({
  title,
  subtitle,
  value,
  options,
  disabled,
  onChange,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  value: string;
  options: (string | { value: string; label: string })[];
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <ActionRow
      title={title}
      subtitle={subtitle}
      trailing={
        <Select
          value={value}
          options={options}
          disabled={disabled}
          ariaLabel={typeof title === "string" ? title : undefined}
          onChange={onChange}
        />
      }
    />
  );
}

/** AdwSpinRow — a −/value/+ stepper with a mono tabular readout. */
export function SpinRow({
  title,
  subtitle,
  value,
  min,
  max,
  step = 1,
  unit,
  disabled = false,
  onChange,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  const set = (v: number) => {
    if (!disabled) onChange(Math.min(max, Math.max(min, v)));
  };
  const stepBtn = (label: string, delta: number) => (
    <button
      type="button"
      disabled={disabled || (delta < 0 ? value <= min : value >= max)}
      onClick={() => set(value + delta)}
      aria-label={`${delta < 0 ? "Decrease" : "Increase"}${typeof title === "string" ? ` ${title}` : ""}`}
      className="flex h-[28px] w-[28px] cursor-pointer items-center justify-center rounded-button bg-white/9 text-[16px] leading-none text-ink disabled:cursor-default disabled:opacity-40"
    >
      {label}
    </button>
  );
  return (
    <ActionRow
      title={title}
      subtitle={subtitle}
      trailing={
        <>
          <span className="numeric min-w-[44px] text-right">
            {value}
            {unit ? ` ${unit}` : ""}
          </span>
          <div className="flex gap-1">
            {stepBtn("−", -step)}
            {stepBtn("+", step)}
          </div>
        </>
      }
    />
  );
}

/** libadwaita switch — pill track, accent fill when on. */
export function Switch({
  checked,
  disabled = false,
  ariaLabel,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      disabled={disabled}
      onClick={() => !disabled && onChange(!checked)}
      className={`relative h-[26px] w-[44px] shrink-0 rounded-full p-0 transition-colors duration-200 disabled:opacity-50 ${
        checked ? "bg-accent-bg" : "bg-white/12"
      } ${disabled ? "cursor-default" : "cursor-pointer"}`}
    >
      <span
        className="absolute top-[3px] h-5 w-5 rounded-full bg-white shadow-[0_1px_2px_rgb(0_0_0/35%)] transition-[left] duration-200"
        style={{ left: checked ? 21 : 3 }}
      />
    </button>
  );
}

export function SwitchRow({
  title,
  subtitle,
  checked,
  disabled,
  onChange,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <ActionRow
      title={title}
      subtitle={subtitle}
      trailing={
        <Switch
          checked={checked}
          disabled={disabled}
          ariaLabel={typeof title === "string" ? title : undefined}
          onChange={onChange}
        />
      }
    />
  );
}
