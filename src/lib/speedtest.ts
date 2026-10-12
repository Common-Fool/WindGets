import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { Text } from "./i18n";

/** Мегабиты в секунду, задержка — в миллисекундах. `null` — этапа не было. */
export type Report = {
  download: number | null;
  upload: number | null;
  latency: number | null;
};

export type Stage = "locate" | "download" | "upload";

export type SpeedtestView =
  | { status: "idle" }
  | { status: "running"; stage: Stage; report: Report }
  | { status: "done"; report: Report }
  | { status: "failed"; reason: string; report: Report };

/**
 * Состояние вместе с прошлыми замерами.
 *
 * История приходит тем же событием: она меняется только вместе с состоянием,
 * и вторая подписка ради неё завела бы два источника правды.
 */
export type Snapshot = SpeedtestView & {
  /** Прошлые замеры, старые в начале. */
  history: Report[];
  /** Город сервера, на котором меряем. */
  server: string | null;
};

type Listener = (snapshot: Snapshot) => void;

let current: Snapshot = { status: "idle", history: [], server: null };
const listeners = new Set<Listener>();
let started = false;

function publish(next: Snapshot) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  // Замер идёт в фоне и переживает закрытие окна: спрашиваем, что там сейчас
  invoke<Snapshot>("speedtest_view")
    .then((initial) => {
      if (current.status === "idle") publish(initial);
    })
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  listen<Snapshot>("speedtest", (event) => publish(event.payload)).catch(() => {
    // Вне Tauri мерить нечем
  });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useSpeedtest(): Snapshot {
  return useSyncExternalStore(subscribe, () => current);
}

/** Запускает замер. Отказ приходит, только если один уже идёт. */
export function runSpeedtest(): Promise<void> {
  return invoke("speedtest_start");
}

/** Что показывать, пока идёт этап. */
export function stageLabel(t: Text, stage: Stage): string {
  return t(`speedtest.stage.${stage}`);
}
