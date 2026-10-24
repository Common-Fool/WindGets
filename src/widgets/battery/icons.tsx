import type { ReactNode } from "react";
import type { DeviceKind } from "../../lib/battery";

/** Контуры периферии в общей сетке значков приложения. */
function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true"
      fill="none" stroke="currentColor" strokeWidth={1.6}
      strokeLinecap="round" strokeLinejoin="round">
      {children}
    </svg>
  );
}

export function DeviceIcon({ kind }: { kind: DeviceKind }) {
  switch (kind) {
    case "mouse":
      return <Glyph><rect x="6" y="3" width="12" height="18" rx="6" /><path d="M12 3v7M6 10h12" /></Glyph>;
    case "keyboard":
      return <Glyph><rect x="2" y="6" width="20" height="12" rx="2" />
        <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M6 14h.01M9 14h9" /></Glyph>;
    case "headset":
      return <Glyph><path d="M4 13v-2a8 8 0 0 1 16 0v6a3 3 0 0 1-3 3h-3" />
        <rect x="3" y="11" width="4" height="7" rx="2" /><rect x="17" y="11" width="4" height="7" rx="2" />
        <path d="M12 20h2" /></Glyph>;
    case "gamepad":
      return <Glyph><path d="M8 7h8c3 0 4 2 5 8 .5 3-1 4-3 2l-2-2H8l-2 2c-2 2-3.5 1-3-2 1-6 2-8 5-8Z" />
        <path d="M8 10v4M6 12h4M16 10h.01M18 12h.01" /></Glyph>;
    case "phone":
      return <Glyph><rect x="6" y="2" width="12" height="20" rx="2" /><path d="M10 5h4M11 19h2" /></Glyph>;
    case "other":
      return <Glyph><rect x="5" y="5" width="14" height="14" rx="3" />
        <path d="M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3M9 9h6v6H9Z" /></Glyph>;
  }
}

/** Молния появляется только при подтверждённой зарядке. */
export function ChargingIcon() {
  return <Glyph><path d="m13 2-8 12h6l-1 8 9-13h-7l1-7Z" /></Glyph>;
}
