import { liveSnapshot } from "./live";

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

/*
 * Подписка одна на окно и снимается, когда слушателей не осталось
 * (`live.ts`): событие приходит раз в две секунды, и держать доставку в окно
 * без карточки незачем.
 */
export const usePing = liveSnapshot<PingState>("ping_state", "ping", { probes: [] });
