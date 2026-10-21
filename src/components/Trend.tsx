import { useId, useRef } from "react";
import { shapeOf, VIEW_WIDTH } from "../lib/chart";
import { useHeight } from "../lib/size";
import type { Accent } from "../lib/accent";
import "./Trend.css";

export type Series = {
  /** Значения от старых к новым. */
  values: readonly number[];
  accent: Accent;
  /** Подпись в легенде под графиком. */
  label: string;
  /**
   * Закрасить место под линией.
   *
   * Заливка идёт не всем рядам: две наложенные друг на друга дают серое пятно
   * вместо графика. Закрашивают главный ряд, остальные остаются линиями.
   */
  fill?: boolean;
  /** Второй признак ряда, сохраняющий различимость в монохроме. */
  dashed?: boolean;
};

type Props = {
  series: Series[];
  /** Потолок: сколько последних отсчётов показывать. */
  capacity?: number;
  /** Шаг горизонтальной сетки в единицах значений. */
  step?: number;
  /** Значения, которые стоит подписать: последние, пиковые, опорные. */
  marks?: number[];
  formatMark?: (value: number) => string;
};

/** Высота системы координат картинки. Наружу она всё равно растягивается. */
const VIEW_HEIGHT = 100;

/**
 * Запас сверху и снизу — доля от размаха значений.
 *
 * Без него наибольший и наименьший замер легли бы ровно на края и читались
 * как обрезанные.
 */
const PADDING = 0.12;

/**
 * Наименьший зазор между подписями в точках.
 *
 * Подписи стоят на своих значениях, а не на делениях сетки, поэтому две
 * близкие цифры налезли бы друг на друга. Нижняя остаётся, верхняя пропадает:
 * так на месте всегда самая мелкая из соседних, а не случайная.
 */
const LABEL_GAP = 13;

/**
 * График на несколько рядов: заливка под каждой линией, лёгкая пунктирная
 * сетка и легенда снизу.
 *
 * Занимает всё место родителя и тянется вместе с окном. Точки раскладываются
 * по числу замеров, а не по потолку: замер здесь — это нажатие кнопки, тридцать
 * штук копятся неделями, и «свежее справа, пусто слева» оставило бы график
 * пустым почти всегда. У истории железа (`Columns`) правило обратное, и там
 * оно верное: отсчёты идут раз в секунду, буфер набирается за пять минут.
 *
 * ВАЖНО: шкала считается **от наименьшего замера, а не от нуля**. Смысл
 * графика — увидеть, как меняется скорость, а не сколько её в абсолюте: от
 * нуля разница между 480 и 570 не видна вовсе. Абсолютные числа стоят рядом
 * подписями и в колонках итогов.
 *
 * Сетка и подписи нарисованы блоками поверх картинки, а не внутри неё:
 * система координат у ломаной условная и тянется по ширине окна, отчего
 * пунктир растянулся бы вместе с ней, а текст поехал бы вбок.
 */
export function Trend({
  series,
  capacity = 30,
  step = 100,
  marks = [],
  formatMark = (value) => value.toFixed(0),
}: Props) {
  const plot = useRef<HTMLDivElement>(null);
  const height = useHeight(plot);

  // Двоеточия из useId в ссылке вида url(#...) не годятся, а имя всё равно
  // должно быть своё: графиков в окне бывает несколько
  const unique = useId().replace(/:/g, "");

  const shown = series.map((row) => ({ ...row, values: row.values.slice(-capacity) }));
  // Сколько замеров есть на самом деле — по ним и раскладываем ширину
  const filled = Math.max(...shown.map((row) => row.values.length), 0);
  const all = shown.flatMap((row) => row.values);

  const { low, high } = bounds(all, step);
  const at = (value: number) => `${((value - low) / (high - low)) * 100}%`;

  const gap = VIEW_WIDTH / Math.max(filled - 1, 1);

  return (
    <div className="trend">
      <div className="trend__plot" ref={plot}>
        <div className="trend__grid" aria-hidden="true">
          {levels(low, high, step).map((value) => (
            <span key={value} className="trend__row" style={{ bottom: at(value) }} />
          ))}
          {Array.from({ length: filled }, (_, index) => (
            <span key={index} className="trend__column" style={{ left: `${index * gap}%` }} />
          ))}
        </div>

        <svg
          className="trend__canvas"
          viewBox={`0 0 ${VIEW_WIDTH} ${VIEW_HEIGHT}`}
          preserveAspectRatio="none"
          aria-hidden="true"
        >
          {shown.map((row) => {
            const shape = shapeOf(row.values, {
              low,
              high,
              capacity: filled,
              height: VIEW_HEIGHT,
              smooth: true,
            });
            if (shape === null) return null;
            const fade = `${unique}-${row.label}`;

            return (
              <g key={row.label} data-accent={row.accent}>
                {row.fill === true && (
                  <>
                    {/*
                     * Заливка гаснет книзу: у самой линии она чуть заметна,
                     * у нижнего края шкалы её нет вовсе. Сплошная читалась бы
                     * закрашенной горой, а не графиком.
                     */}
                    <defs>
                      <linearGradient id={fade} x1="0" y1="0" x2="0" y2="1">
                        <stop className="trend__fade--near" offset="0" />
                        <stop className="trend__fade--far" offset="1" />
                      </linearGradient>
                    </defs>
                    <path className="trend__area" d={shape.area} fill={`url(#${fade})`} />
                  </>
                )}
                {/* Штрих не должен растягиваться вместе с системой координат */}
                <path
                  className={`trend__line${row.dashed ? " trend__line--dashed" : ""}`}
                  d={shape.line}
                  vectorEffect="non-scaling-stroke"
                />
              </g>
            );
          })}
        </svg>

        <div className="trend__scale" aria-hidden="true">
          {readable(marks, low, high, height).map((value) => (
            <span key={value} className="trend__mark" style={{ bottom: at(value) }}>
              {formatMark(value)}
            </span>
          ))}
        </div>
      </div>

      <div className="trend__legend">
        {shown.map((row) => (
          <span key={row.label} className="trend__key" data-accent={row.accent}>
            <span
              className={`trend__sample${row.dashed ? " trend__sample--dashed" : ""}`}
              aria-hidden="true"
            />
            {row.label}
          </span>
        ))}
      </div>
    </div>
  );
}

/**
 * Границы шкалы: от наименьшего значения до наибольшего, с запасом по краям.
 *
 * Одинаковые значения размаха не дают вовсе, и делить на него было бы нечем —
 * тогда разводим границы на одно деление.
 */
function bounds(values: number[], step: number): { low: number; high: number } {
  if (values.length === 0) return { low: 0, high: step };

  const lowest = Math.min(...values);
  const highest = Math.max(...values);

  if (highest === lowest) {
    return { low: lowest - step / 2, high: highest + step / 2 };
  }

  const room = (highest - lowest) * PADDING;
  return { low: lowest - room, high: highest + room };
}

/** Деления сетки: ровные значения внутри шкалы. */
function levels(low: number, high: number, step: number): number[] {
  // Шаг приходит свойством, и нулевой увёл бы цикл ниже в бесконечный —
  // то есть подвесил бы окно целиком
  if (!(step > 0)) return [];

  // Сетка гуще сотни линий не читается, а бесконечные границы не имеют шкалы
  if (!Number.isFinite(low) || !Number.isFinite(high) || (high - low) / step > MAX_LEVELS) return [];

  const first = Math.ceil(low / step) * step;
  const rows: number[] = [];

  for (let value = first; value < high && rows.length < MAX_LEVELS;) {
    rows.push(value);
    // У больших чисел прибавка шага округляется к прежнему значению, даже
    // если размах шкалы небольшой: без продвижения окно зависло бы
    const next = value + step;
    if (!(next > value)) break;
    value = next;
  }
  return rows;
}

/** Больше линий сетки шкала не рисует: гуще их уже не различить. */
const MAX_LEVELS = 100;

/**
 * Оставляет те подписи, что попадают в шкалу и не налезают друг на друга.
 *
 * Вышедшие за края отбрасываем, а не растягиваем ради них шкалу: опорное
 * значение не должно портить картину, когда канал далеко от него.
 *
 * До первого измерения высоты не оставляем ни одной: выбрасывать подписи по
 * нулевой высоте значит выбросить не те.
 */
function readable(marks: number[], low: number, high: number, height: number): number[] {
  if (height <= 0) return [];

  const sorted = [...new Set(marks)]
    .filter((value) => value >= low && value <= high)
    .sort((one, other) => one - other);

  const kept: number[] = [];
  let last = -LABEL_GAP;

  for (const value of sorted) {
    const at = ((value - low) / (high - low)) * height;
    if (at - last < LABEL_GAP) continue;
    kept.push(value);
    last = at;
  }

  return kept;
}
