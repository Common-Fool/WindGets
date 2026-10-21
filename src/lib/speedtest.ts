import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";
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

/*
 * Замер идёт в фоне и переживает закрытие окна, поэтому окно спрашивает, что
 * там сейчас. Подписка одна на окно и снимается, когда слушателей не осталось
 * (`live.ts`); там же порядок — сперва подписка, потом вопрос: финальное
 * событие, ушедшее до слушателя, иначе пропало бы, и карточка стояла бы на
 * «идёт замер» без кнопки запуска.
 */
export const useSpeedtest = liveSnapshot<Snapshot>("speedtest_view", "speedtest", { status: "idle", history: [], server: null });

/** Запускает замер. Отказ приходит, только если один уже идёт. */
export function runSpeedtest(): Promise<void> {
  return invoke("speedtest_start");
}

/** Что показывать, пока идёт этап. */
export function stageLabel(t: Text, stage: Stage): string {
  return t(`speedtest.stage.${stage}`);
}
