import type { ReactNode } from "react";

export type MediaGlyphName = "play" | "pause" | "previous" | "next" | "up" | "down" | "note" | "check";

// Контуры GPT локальны; общие глифы Transport и исходный Claude не меняются.
export function MediaGlyph({ name }: { name: MediaGlyphName }) {
  let glyph: ReactNode;
  switch (name) {
    case "play": glyph = <path d="M8 5 19 12 8 19Z" />; break;
    case "pause": glyph = <><rect x="6.5" y="5" width="4" height="14" rx="1" /><rect x="13.5" y="5" width="4" height="14" rx="1" /></>; break;
    case "previous": glyph = <path d="M6 5v14M18 5 8 12 18 19Z" />; break;
    case "next": glyph = <path d="M18 5v14M6 5 16 12 6 19Z" />; break;
    case "up": glyph = <path d="m6 15 6-6 6 6" />; break;
    case "down": glyph = <path d="m6 9 6 6 6-6" />; break;
    case "check": glyph = <path d="m5 12 4 4 10-10" />; break;
    case "note": glyph = <><path d="M10 17V6l9-2v11M10 8l9-2" /><ellipse cx="7" cy="17" rx="3" ry="2.5" /><ellipse cx="16" cy="15" rx="3" ry="2.5" /></>; break;
  }
  return <svg viewBox="0 0 24 24" width="1em" height="1em" fill="none" stroke="currentColor"
    strokeWidth={1.75} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{glyph}</svg>;
}
