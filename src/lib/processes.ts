import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Group = {
  /** Имя без расширения: «msedge», а не «msedge.exe». */
  name: string;
  /** Проценты от всей машины: сотня — все ядра заняты целиком. */
  cpu: number;
  /** Рабочее множество в байтах, сумма по группе. */
  memory: number;
  /** Сколько процессов с этим именем. Единица в строке не показывается. */
  count: number;
};

/**
 * Оба порядка приходят разом: переключатель ЦПУ/ОЗУ выбирает из уже
 * присланного, а не спрашивает заново. Через команду он ждал бы до двух
 * секунд — столько идёт шаг опроса.
 */
export type ProcessesState = {
  byCpu: Group[];
  byMemory: Group[];
};

const EMPTY: ProcessesState = { byCpu: [], byMemory: [] };

type Listener = (state: ProcessesState) => void;

/*
 * Подписка одна на окно, как у телеметрии, пинга, звука и буфера обмена.
 */
let current: ProcessesState = EMPTY;
const listeners = new Set<Listener>();
let started = false;

function publish(next: ProcessesState) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  // Спрашиваем сразу: событие придёт только со следующим тиком, то есть через
  // две секунды, а окно уже открыто
  invoke<ProcessesState>("processes_state")
    .then((initial) => {
      if (current === EMPTY) publish(initial);
    })
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  listen<ProcessesState>("processes", (event) => publish(event.payload)).catch(() => {
    // Вне Tauri (обычный браузер) процессов просто нет
  });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useProcesses(): ProcessesState {
  return useSyncExternalStore(subscribe, () => current);
}
