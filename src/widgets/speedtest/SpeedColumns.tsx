import type { CSSProperties } from "react";
import type { Report } from "../../lib/speedtest";

/**
 * Наибольшее число замеров в истории.
 *
 * Больше тридцати бэкенд не хранит, но срез держит границу на случай,
 * если в массиве окажется больше.
 */
const CAPACITY = 30;

/**
 * Наименьшая видимая доля высоты столбика.
 *
 * Шкала считается от наименьшего замера по всей истории, а не от нуля:
 * смысл истории — видеть колебания скорости, а не абсолют. Столбик
 * с наименьшим значением оставляет видимый огрызок заливки, а не нулевую полоску.
 */
const MIN_FILL = 0.12;

type Props = {
  /** История замеров, старые в начале. */
  history: readonly Report[];
  /** GPT распределяет места замеров, сохраняя одинаковую толщину всех пар. */
  fit?: boolean;
};

/**
 * История замеров парными столбиками: приём (зелёный) и отдача (розовый) рядом.
 *
 * Не картинка: ряд элементов `<span>` во флексе с высотой заливки в процентах
 * тянется вместе с окном без искажения пропорций `viewBox`.
 *
 * Шкала считается от наименьшего замера по всей истории (по обоим рядам разом),
 * а не от нуля: разница между 480 и 570 видна сразу.
 */
export function SpeedColumns({ history, fit = false }: Props) {
  const entries = history.slice(-CAPACITY);

  const values: number[] = [];
  for (const entry of entries) {
    if (entry.download !== null && Number.isFinite(entry.download)) {
      values.push(entry.download);
    }
    if (entry.upload !== null && Number.isFinite(entry.upload)) {
      values.push(entry.upload);
    }
  }

  const min = values.length > 0 ? Math.min(...values) : 0;
  const max = values.length > 0 ? Math.max(...values) : 0;

  return (
    <div className="speed__columns" data-fit={fit || undefined}
      style={fit ? { "--speed-count": Math.max(1, entries.length) } as CSSProperties : undefined} aria-hidden="true">
      {entries.map((entry, index) => {
        const downShare = shareOf(entry.download, min, max);
        const upShare = shareOf(entry.upload, min, max);

        return (
          <div key={index} className="speed__pair">
            <span className="speed__bar" data-accent="net-down">
              {downShare !== null && (
                <span
                  className="speed__fill"
                  style={{ height: `${(downShare * 100).toFixed(1)}%` }}
                />
              )}
            </span>
            <span className="speed__bar" data-accent="net-up">
              {upShare !== null && (
                <span
                  className="speed__fill"
                  style={{ height: `${(upShare * 100).toFixed(1)}%` }}
                />
              )}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/**
 * Доля высоты заливки для конкретного значения (от MIN_FILL до 1.0).
 *
 * Возвращает `null`, если значение не задано (этап отсутствовал).
 */
function shareOf(value: number | null, min: number, max: number): number | null {
  if (value === null || !Number.isFinite(value)) return null;

  const span = max - min;
  if (span <= 0) return 1;

  const fraction = (value - min) / span;
  const clamped = Math.min(Math.max(fraction, 0), 1);
  return MIN_FILL + (1 - MIN_FILL) * clamped;
}
