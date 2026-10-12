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
 */

export function liveSnapshot<T>(command: string, event: string, empty: T): () => T {
  let current = empty;
  const listeners = new Set<() => void>();
  let started = false;

  function publish(next: T | null) {
    if (next === null) return;
    current = next;
    for (const listener of listeners) listener();
  }

  function start() {
    if (started) return;
    started = true;

    let heard = false;
    invoke<T | null>(command)
      .then((initial) => {
        if (!heard) publish(initial);
      })
      .catch(() => {
        // Вне Tauri данных нет — остаётся пустой снимок
      });

    listen<T>(event, (message) => {
      heard = true;
      publish(message.payload);
    }).catch(() => {});
  }

  function subscribe(listener: () => void): () => void {
    start();
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  return function useSnapshot(): T {
    return useSyncExternalStore(subscribe, () => current);
  };
}
