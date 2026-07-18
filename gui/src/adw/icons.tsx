// Symbolic 16px icons, currentColor, GNOME-style: hand-authored to mirror
// the content they mark rather than pulled from a generic set. They dim
// with their label via the parent's text color.

interface IconProps {
  size?: number;
}

function Svg({ size = 16, children }: IconProps & { children: React.ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden="true">
      {children}
    </svg>
  );
}

/** ⋮ primary-menu dots. */
export function MenuIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <circle cx={8} cy={3} r={1.4} fill="currentColor" />
      <circle cx={8} cy={8} r={1.4} fill="currentColor" />
      <circle cx={8} cy={13} r={1.4} fill="currentColor" />
    </Svg>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M4 4 L12 12 M12 4 L4 12" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round" />
    </Svg>
  );
}

export function PlusIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8 3.5v9M3.5 8h9" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" />
    </Svg>
  );
}

export function TrashIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path
        d="M3.5 4.5h9M6.5 4.5V3h3v1.5M5 4.5l.6 8.5h4.8L11 4.5"
        stroke="currentColor"
        strokeWidth={1.2}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </Svg>
  );
}

/** Warning triangle — failsafe / daemon-unreachable states. */
export function WarnIcon(props: IconProps) {
  return (
    <Svg {...props}>
      <path d="M8 2.5 14 13H2L8 2.5Z" stroke="currentColor" strokeWidth={1.3} strokeLinejoin="round" />
      <path d="M8 6.5v3" stroke="currentColor" strokeWidth={1.4} strokeLinecap="round" />
      <circle cx={8} cy={11.2} r={0.9} fill="currentColor" />
    </Svg>
  );
}
