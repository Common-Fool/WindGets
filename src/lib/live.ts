import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/*
 * Снимок из Rust, который держится свежим сам: первый — командой, дальше —
 * событием. Тот же приём, что у медиа и звука, собранный один раз: у
 * конвертера, загрузчика и их программ он одинаковый до буквы.
 *
 * Сперва подписка, потом вопрос: событие, ушедшее до того, как слушатель
 * встал, иначе пропадало бы, а ответ команды мог быть снят ещё до него — и
 * окно стояло бы на прежнем состоянии до следующей правки («Проверка…» у
 * обновлений навсегда). Порядок ответа и событий всё равно не гарантирован:
 * запоздавший ответ затёр бы свежее событие, поэтому после первого события он
 * молча выбрасывается.
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
  // Ответы и события снятой подписки не должны оживлять её старый снимок
  let generation = 0;

  function publish(next: T | null) {
    if (next === null) return;
    current = next;
    for (const listener of listeners) listener();
  }

  function start() {
    if (unlisten !== null) return;

    const started = ++generation;
    let heard = false;
    const registered = listen<T>(event, (message) => {
      if (generation !== started) return;
      heard = true;
      publish(message.payload);
    });
    unlisten = registered;

    registered
      .then(() => generation === started ? invoke<T | null>(command) : null)
      .then((initial) => {
        if (generation === started && !heard) publish(initial);
      })
      .catch(() => {
        // Вне Tauri данных нет — остаётся пустой снимок
      });
  }

  function stopIfIdle() {
    if (listeners.size > 0 || unlisten === null) return;
    ++generation;
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
