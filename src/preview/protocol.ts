import type { Look } from "../lib/look";
import type { SettingsView } from "../lib/settings";

/*
 * Письма между окном настроек и рамкой превью виджета.
 *
 * Рамка — та же страница виджета (`index.html?widget=<id>&preview`), только
 * вместо IPC Tauri у неё подделка с демоданными (`install.ts`). Всё, что
 * виджет обычно спрашивает у Rust про себя самого — вид, язык, свою строку
 * настроек, — он получает отсюда, от окна настроек.
 */

export const TO_FRAME = "windgets-preview:state";
export const READY = "windgets-preview:ready";
export const HEIGHT = "windgets-preview:height";

/** Что окно настроек знает и рамке передаёт. */
export type PreviewState = {
  look: Look;
  language: string;
  settings: SettingsView;
};

export type ToFrame = { type: typeof TO_FRAME; state: PreviewState };

export type ToHost =
  /** Рамка поставила подделку и ждёт первого состояния. */
  | { type: typeof READY }
  /** Карточка с высотой по содержимому намерила себе высоту. */
  | { type: typeof HEIGHT; height: number };
