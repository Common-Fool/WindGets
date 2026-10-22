import { transformCallback } from "@tauri-apps/api/core";
import { HEIGHT, READY, TO_FRAME, type PreviewState, type ToFrame, type ToHost } from "./protocol";

/*
 * Превью виджета в разделе «Виджеты»: настоящая карточка на демоданных.
 *
 * Окно настроек показывает её в `<iframe>` с адресом
 * `index.html?widget=<id>&preview`, и этот модуль — первый импорт `main.tsx`:
 * он ставит подделку IPC Tauri раньше, чем код хранилищ успеет его позвать.
 *
 * ВАЖНО: **настоящий IPC в рамке есть, и подменить его нельзя.** wry на
 * Windows внедряет скрипты Tauri во все фреймы (флаг «только главный» он не
 * читает), а Tauri объявляет `__TAURI_INTERNALS__` через `defineProperty` —
 * объект не перезаписать и не поправить; первая попытка падала тут на
 * `Object.assign`. Поэтому подделка лежит в своих переменных
 * (`__WINDGETS_PREVIEW_IPC__`, `__WINDGETS_PREVIEW_EVENTS__`), а модули
 * `@tauri-apps/api` смотрят на них первыми — их переписывает плагин
 * `previewIpc` в `vite.config.ts`.
 *
 * Без переписанного API рамка звала бы настоящие команды от имени окна
 * настроек. Поэтому перед установкой — проверка, что переписанный код на
 * месте; нет — исключение, и страница рамки не грузится вовсе.
 *
 * Сама подделка крошечная и есть в каждом окне, а демоданные приезжают
 * `import()` при первом вопросе — настоящие окна их не грузят.
 *
 * В разработке `?preview` работает и без рамки, с видом и настройками по
 * умолчанию: так демо каждого виджета проверяется в браузере
 * (`localhost:1420/?widget=mail&preview`).
 */

type Callback = (payload: unknown) => void;

type Listener = { event: string; handler: number };

const params = new URLSearchParams(window.location.search);
const framed = window.top !== window;

declare global {
  interface Window {
    __WINDGETS_PREVIEW_IPC__?: unknown;
    __WINDGETS_PREVIEW_EVENTS__?: unknown;
  }
}

if (params.has("preview") && (framed || import.meta.env.DEV)) {
  if (!String(transformCallback).includes("__WINDGETS_PREVIEW_IPC__")) {
    throw new Error("превью: API Tauri не переписан плагином previewIpc — рамка не грузится");
  }
  install(params.get("widget") ?? "");
}

function install(id: string): void {
  const callbacks = new Map<number, Callback>();
  const listeners = new Map<number, Listener>();
  let next = 0;

  function emit(event: string, payload: unknown): void {
    for (const [eventId, listener] of listeners) {
      if (listener.event !== event) continue;
      callbacks.get(listener.handler)?.({ event, id: eventId, payload });
    }
  }

  // Первое состояние даёт окно настроек; без рамки — демо по умолчанию
  let adopt: (state: PreviewState) => void = () => {};
  const first = new Promise<PreviewState>((resolve) => {
    adopt = resolve;
  });
  let current: PreviewState | null = null;

  function tell(message: ToHost): void {
    if (framed) window.parent.postMessage(message, window.location.origin);
  }

  if (framed) {
    window.addEventListener("message", (event: MessageEvent<ToFrame>) => {
      if (event.origin !== window.location.origin || event.source !== window.parent) return;
      if (event.data?.type !== TO_FRAME) return;

      const state = event.data.state;
      const before = current;
      current = state;
      if (before === null) {
        adopt(state);
        return;
      }
      if (before.look !== state.look) emit("appearance", state.look);
      if (before.language !== state.language) emit("language", state.language);
      if (before.settings !== state.settings) emit("settings", state.settings);
    });
  } else {
    void import("./demo").then((demo) => {
      current = demo.defaultState(id);
      adopt(current);
    });
  }

  const demo = import("./demo");

  async function invoke(command: string, args: Record<string, unknown> = {}): Promise<unknown> {
    switch (command) {
      case "plugin:event|listen": {
        const eventId = ++next;
        listeners.set(eventId, { event: String(args.event), handler: Number(args.handler) });
        return eventId;
      }
      case "plugin:event|unlisten":
        listeners.delete(Number(args.eventId));
        return null;
      case "fit_window_height":
        tell({ type: HEIGHT, height: Number(args.height) });
        return null;
    }

    const [{ answer }, state] = await Promise.all([demo, first]);
    // Копия: виджет вправе держать ответ у себя, а следующий вопрос получит свой
    const reply = answer(command, args, { id, state: current ?? state, emit });
    return reply === undefined ? null : structuredClone(reply);
  }

  Object.assign(window, {
    __WINDGETS_PREVIEW_IPC__: {
      invoke,
      transformCallback(callback: Callback, once = false) {
        const key = ++next;
        callbacks.set(key, once ? (payload) => { callbacks.delete(key); callback(payload); } : callback);
        return key;
      },
      unregisterCallback(key: number) {
        callbacks.delete(key);
      },
      convertFileSrc: (path: string) => path,
      metadata: {
        currentWindow: { label: id },
        currentWebview: { windowLabel: id, label: id },
      },
    },
    __WINDGETS_PREVIEW_EVENTS__: {
      unregisterListener(_event: string, eventId: number) {
        listeners.delete(eventId);
      },
    },
  });

  tell({ type: READY });
}
