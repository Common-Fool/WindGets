import type { ReactNode } from "react";
import type { Reading } from "../../lib/telemetry";
import { showReading } from "../../lib/format";
import { MetricPair } from "../../components/MetricPair";

/** Ячейка ряда: показание, его текст (если не по умолчанию) и подсказка. */
export type Cell = {
  /** `undefined` — роли нет, датчика на этом железе не бывает. */
  reading: Reading | undefined;
  /** Своя запись значения — у частоты памяти NVIDIA она удвоена. */
  text?: string;
  hint: string;
};

type Props = {
  /** Снимок пришёл: только тогда отсутствие роли что-то значит. */
  live: boolean;
  label: string;
  icon: ReactNode;
  cells: Cell[];
};

/**
 * Ряд карточки железа, собранный из того, что есть на этом железе (этап 57).
 *
 * Ячейка без роли уходит, и оставшаяся встаёт в левый столбец с подсказкой
 * при себе: у платы без процессорного вентилятора «chassis» не повиснет под
 * пустым местом. Ушли обе — уходит весь ряд. Роль есть, а значения нет —
 * прочерк, как и прежде: «датчик есть, значения нет».
 *
 * Пока снимка нет, ряд стоит целиком с прочерками: убери его сразу, и на
 * подключении карточка собиралась бы у всех на глазах.
 */
export function MetricLine({ live, label, icon, cells }: Props) {
  const shown = live ? cells.filter((cell) => cell.reading !== undefined) : cells;
  const [first, second] = shown;
  if (first === undefined) return null;

  const text = (cell: Cell) => cell.text ?? showReading(cell.reading);

  return (
    <MetricPair
      label={label}
      icon={icon}
      first={text(first)}
      firstHint={first.hint}
      second={second === undefined ? undefined : text(second)}
      secondHint={second?.hint}
      solo={second === undefined}
    />
  );
}
