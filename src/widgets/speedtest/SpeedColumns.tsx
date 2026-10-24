import { useRef, type CSSProperties } from "react";
import type { Report } from "../../lib/speedtest";
import { useWidth } from "../../lib/size";
import { DownArrow, UpArrow } from "../../components/Arrows";

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

// Целые CSS-пиксели: толщина и просветы не меняются с кеглем карточки.
const BAR_WIDTH = 3;
const INNER_GAP = 1;
const PAIR_GAP = 3;
const ROW_GAP = 1;
const PAIR_WIDTH = BAR_WIDTH * 2 + INNER_GAP;
const GEOMETRY = {
  "--speed-bar": `${BAR_WIDTH}px`,
  "--speed-inner-gap": `${INNER_GAP}px`,
} as CSSProperties;

type Props = {
  /** История замеров, старые в начале. */
  history: readonly Report[];
  /** Пары рядом или два отдельных ряда с общими позициями замеров. */
  variant?: "pairs" | "rows";
};

/**
 * История замеров: пары с промежутками или два отдельных ряда.
 *
 * Толщина обоих столбиков одинакова. Шаг постоянный, свежее у правого края.
 * Если места не хватает, показываем последние
 * целые замеры, без усреднения, сжатия столбиков и прокрутки. История на диске
 * остаётся полной; шкала обоих вариантов считается по всем тридцати замерам.
 *
 * Шкала считается от наименьшего замера по всей истории (по обоим рядам разом),
 * а не от нуля: разница между 480 и 570 видна сразу.
 */
export function SpeedColumns({ history, variant = "pairs" }: Props) {
  const row = useRef<HTMLDivElement>(null);
  const width = Math.floor(useWidth(row));
  const entries = history.slice(-CAPACITY);
  const separate = variant === "rows";
  const itemWidth = separate ? BAR_WIDTH : PAIR_WIDTH;
  const gap = separate ? ROW_GAP : PAIR_GAP;
  const capacity = Math.max(0, Math.floor((width + gap) / (itemWidth + gap)));
  const visible = capacity > 0 ? entries.slice(-capacity) : [];

  /*
   * Шаг постоянный, как у обычного графика: будто история уже полная. Свежий
   * замер стоит у правого края, старые уезжают влево по мере новых. Прежде
   * остаток ширины делился между тем, что есть, и три замера стояли по краям
   * и посередине с огромными пропусками (22 октября, слово пользователя).
   * Шаг может быть дробным, а левый край каждого столбика — всегда целый.
   */
  const slots = Math.min(CAPACITY, capacity);
  const step = slots > 1 ? (width - itemWidth) / (slots - 1) : 0;
  const leftOf = (index: number) => Math.round(width - itemWidth - (visible.length - 1 - index) * step);

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
    <div className="speed__columns" data-variant={variant} ref={row} style={GEOMETRY} aria-hidden="true">
      {separate ? (
        (["download", "upload"] as const).map((direction) => (
          <div className="speed__lane" key={direction}>
            <span className="speed__lane-label" data-accent={direction === "download" ? "net-down" : "net-up"}>
              {direction === "download" ? <DownArrow /> : <UpArrow />}
              {direction === "download" ? "Download" : "Upload"}
            </span>
            <div className="speed__lane-bars">
              {visible.map((entry, index) => (
                <SpeedBar key={index} accent={direction === "download" ? "net-down" : "net-up"}
                  share={shareOf(entry[direction], min, max)} left={leftOf(index)} />
              ))}
            </div>
          </div>
        ))
      ) : visible.map((entry, index) => (
        <div key={index} className="speed__pair" style={{ left: `${leftOf(index)}px` }}>
          <SpeedBar accent="net-down" share={shareOf(entry.download, min, max)} />
          <SpeedBar accent="net-up" share={shareOf(entry.upload, min, max)} />
        </div>
      ))}
    </div>
  );
}

function SpeedBar({ accent, share, left }: { accent: "net-down" | "net-up"; share: number | null; left?: number }) {
  return (
    <span className="speed__bar" data-accent={accent} style={left === undefined ? undefined : { left: `${left}px` }}>
      {share !== null && <span className="speed__fill" style={{ height: `${(share * 100).toFixed(1)}%` }} />}
    </span>
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
