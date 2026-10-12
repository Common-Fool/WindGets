/*
 * Значки виджета звука. Рисуем сами, как и на пульте: ради шести глифов тянуть
 * шрифт или библиотеку незачем, а так они наследуют цвет от кнопки и растут
 * вместе с кеглем.
 *
 * Заливку корпуса включает оформление референса через sound-icon__solid.
 * Полоса заглушения входит в сам SVG и растёт вместе с ним.
 */

import type { ReactNode } from "react";
import type { Kind } from "../../lib/sound";

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

/** Корпус динамика — общий у звучащего и у молчащего. */
const CONE = <path className="sound-icon__solid" d="M4 9.5h3.2L12 5.5v13l-4.8-4H4a.5.5 0 0 1-.5-.5v-4a.5.5 0 0 1 .5-.5Z" />;

export function SpeakerIcon() {
  return (
    <Glyph>
      {CONE}
      {/* Две волны, а не три: на мелком кегле третья сливается со второй */}
      <path d="M15.5 9.2a4 4 0 0 1 0 5.6" />
      <path d="M18 6.8a7.5 7.5 0 0 1 0 10.4" />
    </Glyph>
  );
}

export function ChevronIcon({ up = false, right = false }: { up?: boolean; right?: boolean }) {
  return <Glyph><path d={right ? "m9 6 6 6-6 6" : up ? "m6 15 6-6 6 6" : "m6 9 6 6 6-6"} /></Glyph>;
}

export function CycleIcon() {
  return <Glyph><path d="M4 10a8 8 0 0 1 13.7-5.7L20 7M20 3v4h-4M20 14a8 8 0 0 1-13.7 5.7L4 17M4 21v-4h4" /></Glyph>;
}

export function MixerIcon() {
  return <Glyph><path d="M6 3v5m0 4v9M12 3v10m0 4v4M18 3v2m0 4v12M3.5 8h5v4h-5ZM9.5 13h5v4h-5ZM15.5 5h5v4h-5Z" /></Glyph>;
}

export function SpeakerMutedIcon() {
  return (
    <Glyph>
      {CONE}
      <path d="m3 3 18 18" />
    </Glyph>
  );
}

export function MicrophoneIcon() {
  return (
    <Glyph>
      <rect className="sound-icon__solid" x="9" y="3" width="6" height="10.5" rx="3" />
      <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0" />
      <path d="M12 18v3m-3 0h6" />
    </Glyph>
  );
}

export function MicrophoneMutedIcon() {
  return (
    <Glyph>
      <rect className="sound-icon__solid" x="9" y="3" width="6" height="10.5" rx="3" />
      <path d="M5.5 11.5a6.5 6.5 0 0 0 13 0" />
      <path d="M12 18v3m-3 0h6" />
      {/* Косая через весь значок: у микрофона нет места для крестика сбоку */}
      <path d="M4 4 20 20" />
    </Glyph>
  );
}

function HeadphonesIcon() {
  return (
    <Glyph>
      <path d="M4 14v-2a8 8 0 0 1 16 0v2" />
      <rect x="2.5" y="13.5" width="4.5" height="7" rx="2" />
      <rect x="17" y="13.5" width="4.5" height="7" rx="2" />
    </Glyph>
  );
}

function DisplayIcon() {
  return (
    <Glyph>
      <rect x="2.5" y="4" width="19" height="13" rx="2" />
      <path d="M9 20.5h6" />
      <path d="M12 17v3.5" />
    </Glyph>
  );
}

function BoxIcon() {
  return (
    <Glyph>
      <rect x="4.5" y="3" width="15" height="18" rx="2" />
      <circle cx="12" cy="14" r="3.5" />
      <circle cx="12" cy="7" r="1.5" />
    </Glyph>
  );
}

/** Значок устройства по его типу — то же деление, что в системном флайауте. */
export function DeviceIcon({ kind }: { kind: Kind }) {
  switch (kind) {
    case "headphones":
      return <HeadphonesIcon />;
    case "display":
      return <DisplayIcon />;
    case "speakers":
      return <BoxIcon />;
    default:
      return <SpeakerIcon />;
  }
}
