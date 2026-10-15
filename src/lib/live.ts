import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/*
 * Снимок из Rust, который держится свежим сам: первый — командой, дальше —
 * событием. Тот же приём, что у медиа и звука, собранный один раз: у
 * конвертера, загрузчика и их программ он одинаковый до буквы.
 *
 * Спрашиваем и подписываемся разом, и порядок ответов не гарантирован:
 * запоздавший ответ команды затёр бы свежее событие, поэтому после первого
 * события он молча выбрасывается.
 *
 * Подписка снимается, когда в окне не осталось слушателей, — как у
 * телеметрии (`telemetry.ts`): страница настроек, раз открытая, иначе держала
 * бы доставку событий в спрятанное окно до выхода. Следующий слушатель
 * спрашивает снимок заново: запомненный к тому времени мог устареть.
 */

export function liveSnapshot<T>(command: string, event: string, empty: T): () => T {
  let current = empty;
  const listeners = new Set<() => void>();
  /** Снятие подписки на событие; `null` — окно поток не слушает. */
  let unlisten: Promise<() => void> | null = null;

  function publish(next: T | null) {
    if (next === null) return;
    current = next;
    for (const listener of listeners) listener();
  }

  function start() {
    if (unlisten !== null) return;

    let heard = false;
    invoke<T | null>(command)
      .then((initial) => {
        if (!heard) publish(initial);
      })
      .catch(() => {
        // Вне Tauri данных нет — остаётся пустой снимок
      });

    unlisten = listen<T>(event, (message) => {
      heard = true;
      publish(message.payload);
    });
    unlisten.catch(() => {});
  }

  function stopIfIdle() {
    if (listeners.size > 0 || unlisten === null) return;
    const pending = unlisten;
    unlisten = null;
    pending.then((stop) => stop()).catch(() => {});
  }

  function subscribe(listener: () => void): () => void {
    start();
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
      // Не сразу: `StrictMode` и смена раздела снимают и тут же заводят
      // подписку заново, и снимать доставку на этот миг незачем
      setTimeout(stopIfIdle, 1000);
    };
  }

  return function useSnapshot(): T {
    return useSyncExternalStore(subscribe, () => current);
  };
}
