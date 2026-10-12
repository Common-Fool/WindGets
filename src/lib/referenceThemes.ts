import type { Key } from "./i18n";

/*
 * Подписи общих тем и GPT для карточек выбора в окне настроек.
 *
 * Список, порядок и имена приходят из Rust (`settings::PALETTES`), цвета — из
 * tokens.css. Здесь только то, что читает человек: строка о том, как цвета
 * разложены по показателям, — ключом каталога строк.
 */
export const REFERENCE_THEMES: Record<string, { description: Key }> = {
  gpt: { description: "theme.gpt" },
  "reference-classic": { description: "theme.classic" },
  "reference-polar": { description: "theme.polar" },
  "reference-indigo": { description: "theme.indigo" },
  "reference-mint": { description: "theme.mint" },
  "reference-spectrum": { description: "theme.spectrum" },
  "reference-a": { description: "theme.a" },
  "reference-b": { description: "theme.b" },
  "reference-c": { description: "theme.c" },
};

/** Тема, а не режим: у режимов своих описаний и оформления нет. */
export function isReferenceTheme(id: string): boolean {
  return Object.hasOwn(REFERENCE_THEMES, id);
}

/** Фактические роли управления, данных, состояний, паролей и погоды. */
export const REFERENCE_COLOR_ROLES = [
  { id: "control", usage: "role.control", token: "--ui-accent" },
  { id: "cpu", usage: "role.cpu", token: "--metric-cpu" },
  { id: "ram", usage: "role.ram", token: "--metric-ram" },
  { id: "gpu", usage: "role.gpu", token: "--metric-gpu" },
  { id: "storage", usage: "role.storage", token: "--metric-storage" },
  { id: "media", usage: "role.media", token: "--metric-media" },
  { id: "down", usage: "role.down", token: "--metric-net-down" },
  { id: "up", usage: "role.up", token: "--metric-net-up" },
  { id: "ok", usage: "role.ok", token: "--status-ok" },
  { id: "warn", usage: "role.warn", token: "--status-warn" },
  { id: "danger", usage: "role.danger", token: "--status-danger" },
  { id: "lower", usage: "role.lower", token: "--char-lower" },
  { id: "upper", usage: "role.upper", token: "--char-upper" },
  { id: "digit", usage: "role.digit", token: "--char-digit" },
  { id: "symbol", usage: "role.symbol", token: "--char-symbol" },
  { id: "warm", usage: "role.warm", token: "--sky-warm" },
  { id: "water", usage: "role.water", token: "--sky-water" },
] as const satisfies readonly { id: string; usage: Key; token: string }[];
