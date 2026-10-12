import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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

type Listener = (state: ClipboardState) => void;

/*
 * Подписка одна на окно, как у телеметрии, пинга и звука. Событие тут приходит
 * реже всех — только на смену содержимого буфера, — но заводить по listen на
 * каждый вызов хука незачем и здесь.
 */
let current: ClipboardState = EMPTY;
const listeners = new Set<Listener>();
let started = false;

function publish(next: ClipboardState) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  // Спрашиваем сразу: список уже полон, а событие придёт только со следующим
  // копированием — то есть неизвестно когда
  invoke<ClipboardState>("clipboard_state")
    .then((initial) => {
      if (current === EMPTY) publish(initial);
    })
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  listen<ClipboardState>("clipboard", (event) => publish(event.payload)).catch(() => {
    // Вне Tauri (обычный браузер) буфера просто нет
  });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useClipboard(): ClipboardState {
  return useSyncExternalStore(subscribe, () => current);
}

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
