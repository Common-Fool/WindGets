import { useSyncExternalStore } from "react";
import { subscribe } from "./telemetry";

/**
 * Пять минут при опросе раз в секунду.
 *
 * Длина буфера — это ШАГ СТОЛБИКА, а не только глубина памяти: столбиков в
 * ряду три десятка, и на двух минутах на каждый приходилось по четыре
 * секунды. Столбик в четыре секунды показывает рябь, а не ход загрузки —
 * отсюда пять минут и десять секунд на столбик.
 *
 * Считается в отсчётах, а не в секундах: на шаге опроса в две секунды буфер
 * растягивается вдвое вместе с шагом столбика.
 */
export const HISTORY_LENGTH = 300;

const EMPTY: readonly number[] = Object.freeze([]);

/*
 * История копится только по рядам, которые хоть раз попросили, — с этого
 * момента и до закрытия окна. Прежде копились все двести с лишним датчиков, и
 * каждую секунду в каждом окне железа пересобиралось шестьдесят тысяч чисел
 * ради восьми рядов, которые рисуются (замер 11 октября). Попрошенный ряд не
 * забывается и при смене вида: вернувшийся вид получает накопленное.
 */
const series = new Map<string, readonly number[]>();
const tracked = new Set<string>();
const listeners = new Set<() => void>();

/**
 * Метка времени последнего учтённого снимка. Копим именно по ней, а не по
 * перерисовке: StrictMode в разработке монтирует дважды, и без этой проверки
 * каждая точка попадала бы в буфер по два раза.
 */
let lastAt = 0;

subscribe((state) => {
  if (state.status !== "ok") return;
  if (state.snapshot.at === lastAt) return;
  lastAt = state.snapshot.at;

  for (const reading of state.snapshot.readings) {
    if (reading.value === null || !tracked.has(reading.path)) continue;

    const previous = series.get(reading.path) ?? EMPTY;
    const trimmed =
      previous.length >= HISTORY_LENGTH
        ? previous.slice(previous.length - HISTORY_LENGTH + 1)
        : previous;
    series.set(reading.path, [...trimmed, reading.value]);
  }

  for (const listener of listeners) listener();
});

function subscribeToHistory(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Ряд значений датчика за последние пять минут, от старых к новым.
 * Пустой массив означает, что данных ещё не накопилось.
 */
export function useSeries(path: string | undefined): readonly number[] {
  if (path) tracked.add(path);
  return useSyncExternalStore(subscribeToHistory, () =>
    path ? (series.get(path) ?? EMPTY) : EMPTY,
  );
}
