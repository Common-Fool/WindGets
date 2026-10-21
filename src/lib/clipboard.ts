import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";

export type Entry = {
  id: number;
  /** Одна строка для показа: пробелы сжаты, длина обрезана. */
  preview: string;
  /** Предпросмотр GPT: переводы строк сохранены, предел прежний — 200 знаков. */
  formattedPreview: string;
  /** Число исходных строк, включая пустые; CRLF считается одним переводом. */
  lines: number;
  /** Сколько знаков в записи целиком — для подсказки под курсором. */
  chars: number;
  /** Эта запись лежит в буфере прямо сейчас. */
  current: boolean;
};

export type ClipboardState = {
  /** Свежая первой. */
  entries: Entry[];
  /** Сколько записей помещается всего. */
  capacity: number;
};

export type Action =
  | { kind: "copy"; id: number }
  | { kind: "remove"; id: number }
  | { kind: "clear" };

const EMPTY: ClipboardState = { entries: [], capacity: 0 };

/*
 * Подписка одна на окно и снимается, когда слушателей не осталось
 * (`live.ts`); там же порядок — сперва подписка, потом снимок: событие,
 * ушедшее до слушателя, иначе пропало бы до следующего копирования.
 */
export const useClipboard = liveSnapshot<ClipboardState>("clipboard_state", "clipboard", EMPTY);

export type ControlFailure = "unavailable" | "missing" | "copyFailed" | "clearFailed";
export type ControlResult = { ok: true } | { ok: false; reason: ControlFailure };

/** Ждёт исполнения в рабочем потоке; отказ возвращает значением, без rejection. */
export async function controlConfirmed(action: Action): Promise<ControlResult> {
  try {
    await invoke<void>("clipboard_control", { action });
    return { ok: true };
  } catch (error: unknown) {
    console.warn("буфер не послушался", error);
    const reason = error === "missing" || error === "copyFailed" || error === "clearFailed"
      ? error : "unavailable";
    return { ok: false, reason };
  }
}

/** Прежние обработчики не ждут ответа; controlConfirmed уже перехватил отказ. */
export function control(action: Action): void {
  void controlConfirmed(action);
}
