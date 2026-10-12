import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Что намеряли по цели. */
export type Reading = {
  /** Миллисекунды. `null` — не ответил. */
  latency: number | null;
  /** Разброс соседних откликов в миллисекундах. `null` — ответов пока мало. */
  jitter: number | null;
  /** Доля потерь в последних попытках, от нуля до единицы. */
  loss: number;
};

/** Одна цель. */
export type Probe = {
  /** Имя оператора — оно и стоит в карточке. */
  name: string;
  /** Адрес: живёт в подсказке. */
  target: string;
  /** `null` — первой попытки ещё не было; это не «не ответил». */
  reading: Reading | null;
};

export type PingState = {
  probes: Probe[];
};

type Listener = (state: PingState) => void;

/*
 * Подписка одна на окно, как у телеметрии: событие приходит раз в две секунды,
 * и заводить по listen на каждый вызов хука незачем.
 */
let current: PingState = { probes: [] };
const listeners = new Set<Listener>();
let started = false;

function publish(next: PingState) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  // Спрашиваем сразу, чтобы окно, открытое между тиками, не ждало впустую
  invoke<PingState>("ping_state")
    .then((initial) => {
      if (current.probes.length === 0) publish(initial);
    })
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  listen<PingState>("ping", (event) => publish(event.payload)).catch(() => {
    // Вне Tauri (обычный браузер) замеров просто нет
  });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function usePing(): PingState {
  return useSyncExternalStore(subscribe, () => current);
}
