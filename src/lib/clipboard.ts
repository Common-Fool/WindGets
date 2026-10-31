import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";

export type Entry = {
  id: number;
  /** Одна строка для показа: пробелы сжаты, длина обрезана. */
  preview: string;
  /** Предпросмотр NEW: переводы строк сохранены, предел прежний — 200 знаков. */
  formattedPreview: string;
  /** Число исходных строк, включая пустые; CRLF считается одним переводом. */
  lines: number;
  /** Сколько знаков в записи целиком — для подсказки под курсором. */
  chars: number;
  /** Эта запись лежит в буфере прямо сейчас. */
  current: boolean;
  /** Закреплена: стоит над историей, не вытесняется и переживает «Очистить». */
  pinned: boolean;
};

/** Картинка без пикселей; миниатюру даёт `clipboardThumb`. */
export type ImageEntry = {
  id: number;
  width: number;
  height: number;
  /** Эта картинка лежит в буфере прямо сейчас. */
  current: boolean;
};

export type ClipboardState = {
  /** Закреплённые первыми, за ними история, свежая первой. */
  entries: Entry[];
  /** Сколько незакреплённых записей помещается всего. */
  capacity: number;
  /** Свежая первой. */
  images: ImageEntry[];
  imageCapacity: number;
};

export type ClearTarget = "text" | "images";

export type Action =
  | { kind: "copy"; id: number }
  | { kind: "remove"; id: number }
  | { kind: "pin"; id: number; pinned: boolean }
  | { kind: "copyImage"; id: number }
  | { kind: "removeImage"; id: number }
  /** Окно сохранения; ответ приходит, когда файл записан или выбор отменён. */
  | { kind: "saveImage"; id: number }
  | { kind: "clear"; what?: ClearTarget };

const EMPTY: ClipboardState = { entries: [], capacity: 0, images: [], imageCapacity: 0 };

/** Миниатюра картинки, `data:`-адрес; `null` — картинки уже нет. */
export function clipboardThumb(id: number): Promise<string | null> {
  return invoke<string | null>("clipboard_thumb", { id }).catch(() => null);
}

/*
 * Подписка одна на окно и снимается, когда слушателей не осталось
 * (`live.ts`); там же порядок — сперва подписка, потом снимок: событие,
 * ушедшее до слушателя, иначе пропало бы до следующего копирования.
 */
export const useClipboard = liveSnapshot<ClipboardState>("clipboard_state", "clipboard", EMPTY);

export type ControlFailure = "unavailable" | "missing" | "copyFailed" | "clearFailed" | "pinsFull" | "saveFailed";
export type ControlResult = { ok: true } | { ok: false; reason: ControlFailure };

/** Ждёт исполнения в рабочем потоке; отказ возвращает значением, без rejection. */
export async function controlConfirmed(action: Action): Promise<ControlResult> {
  try {
    await invoke<void>("clipboard_control", { action });
    return { ok: true };
  } catch (error: unknown) {
    console.warn("буфер не послушался", error);
    const known: ControlFailure[] = ["missing", "copyFailed", "clearFailed", "pinsFull", "saveFailed"];
    const reason = known.find((failure) => failure === error) ?? "unavailable";
    return { ok: false, reason };
  }
}

/** Прежние обработчики не ждут ответа; controlConfirmed уже перехватил отказ. */
export function control(action: Action): void {
  void controlConfirmed(action);
}
