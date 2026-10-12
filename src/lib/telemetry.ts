import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * Единицы уже приведены к базовым на стороне Rust.
 * LHM отдаёт «88,3 KB/s», а при росте — «38,7 MB/s»; если не нормализовать,
 * график прыгнет в тысячу раз на ровном месте.
 */
export type Unit =
  | "celsius"
  | "percent"
  | "watt"
  | "megahertz"
  | "volt"
  | "rpm"
  | "byte"
  | "bytePerSecond"
  | "nanosecond"
  | "raw";

export type Reading = {
  /** Полный путь по дереву — единственный надёжный ключ датчика. */
  path: string;
  /** Последний сегмент пути, например «Core (Tctl/Tdie)». */
  name: string;
  /** Идентификатор LHM. Справочно: он НЕ уникален, ключом быть не может. */
  sensorId: string;
  /** Тип датчика по версии LHM: Temperature, Load, Throughput и так далее. */
  kind: string;
  /** null означает «датчик есть, значения нет». Ноль сюда не подставляем. */
  value: number | null;
  unit: Unit;
};

/** Вторая температура видеокарты и что она такое. */
export type Heat = { path: string; kind: "memory" | "hotspot" };

/**
 * Какой датчик что значит на этом железе — считает Rust (`lhm/roles.rs`).
 * `null` — такого датчика тут нет вовсе, и его ячейка уходит из карточки.
 * Форма повторяет `Roles` в Rust.
 */
export type Roles = {
  cpu: {
    load: string | null;
    /** Один путь или частоты всех ядер — тогда показывается среднее. */
    clock: string[];
    temperature: string | null;
    power: string | null;
    voltage: string | null;
  };
  board: {
    cpuFan: string | null;
    chassisFan: string | null;
    temperature: string | null;
  };
  /** `null` — дискретной видеокарты нет. */
  gpu: {
    name: string;
    vendor: "nvidia" | "amd" | "intel";
    load: string | null;
    memoryLoad: string | null;
    memoryUsed: string | null;
    memoryTotal: string | null;
    temperature: string | null;
    secondTemperature: Heat | null;
    /** Не больше двух. */
    fans: string[];
    clock: string | null;
    memoryClock: string | null;
    power: string | null;
    voltage: string | null;
  } | null;
};

/** Датчики для ручного выбора в настройках процессора — полные пути. */
export type Choices = {
  fans: string[];
  temperatures: string[];
  /** И процессора, и чипа платы. */
  voltages: string[];
};

export type Snapshot = {
  /** Момент снятия, unix-время в миллисекундах. */
  at: number;
  readings: Reading[];
  roles: Roles;
  choices: Choices;
};

export type SourceState =
  | { status: "connecting" }
  /** Модули железа выключены: сайдкар не запущен вовсе. */
  | { status: "idle" }
  | { status: "ok"; snapshot: Snapshot }
  | { status: "unavailable"; reason: string };

type Listener = (state: SourceState) => void;

/*
 * Подписка одна на всё окно. Раньше её заводил каждый вызов хука — при одном
 * виджете это было незаметно, но у панели секций с десяток, и каждая держала
 * бы собственный listen на один и тот же поток.
 */
let current: SourceState = { status: "connecting" };
const listeners = new Set<Listener>();

/**
 * Снятие подписки на событие; `null` — окно поток не слушает.
 *
 * ВАЖНО: **подписка снимается, когда в окне не осталось слушателей.** Прежде
 * она заводилась навсегда, и окно настроек, раз открывшее «Виджеты» или
 * «Данные», дальше круглые сутки спрятанным разбирало снимок из двухсот с
 * лишним датчиков каждую секунду. Tauri шлёт событие только окнам, у которых
 * есть слушатель, — снятая подписка снимает и саму доставку.
 */
let unlisten: Promise<() => void> | null = null;

/** Пришло ли событие после нынешнего запуска: тогда ответ на запрос старше. */
let fresh = false;

function publish(next: SourceState) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (unlisten !== null) return;
  fresh = false;

  // Спрашиваем текущее состояние сразу, чтобы не ждать первого тика — и после
  // перерыва тоже: запомненный снимок к тому времени устарел. Если событие
  // успело прийти раньше ответа — оно свежее, не затираем.
  invoke<SourceState>("telemetry_state")
    .then((initial) => {
      if (!fresh) publish(initial);
    })
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  unlisten = listen<SourceState>("telemetry", (event) => {
    fresh = true;
    publish(event.payload);
  });
  unlisten.catch(() => {
    // Вне Tauri (обычный браузер) потока телеметрии просто нет
  });
}

function stopIfIdle() {
  if (listeners.size > 0 || unlisten === null) return;
  const pending = unlisten;
  unlisten = null;
  pending.then((stop) => stop()).catch(() => {});
}

export function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    // Не сразу: `StrictMode` и смена раздела снимают и тут же заводят
    // подписку заново, и снимать доставку на этот миг незачем
    setTimeout(stopIfIdle, 1000);
  };
}

export function getState(): SourceState {
  return current;
}

/** Текущее состояние источника. Снимки приходят раз в секунду. */
export function useTelemetry(): SourceState {
  return useSyncExternalStore(subscribe, getState);
}

/** Показание по полному пути или undefined, если такого датчика нет. */
export function findReading(state: SourceState, path: string): Reading | undefined {
  if (state.status !== "ok") return undefined;
  return state.snapshot.readings.find((reading) => reading.path === path);
}

/**
 * Показание по хвосту пути, например «/Temperatures/GPU Core».
 *
 * Первый сегмент полного пути — имя машины, и завязываться на него нельзя:
 * переименование компьютера разом сломало бы все селекторы.
 */
export function findByPathEnd(state: SourceState, suffix: string): Reading | undefined {
  if (state.status !== "ok") return undefined;
  return state.snapshot.readings.find((reading) => reading.path.endsWith(suffix));
}
