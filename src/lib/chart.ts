/*
 * Геометрия ломаной. Общая у графика с одним рядом и у графика с несколькими:
 * правило «свежая точка у правого края» и способ считать координаты должны
 * совпадать, иначе два графика в соседних окнах будут читаться по-разному.
 */

/** Внутренняя ширина системы координат; наружу растягивается по месту. */
export const VIEW_WIDTH = 100;

/**
 * Натяжение сглаживающей кривой.
 *
 * Единица — это Catmull-Rom как он есть, и на резких скачках он вылезает за
 * крайние точки. Три четверти сглаживают углы, но горб выше наибольшего
 * замера не рисуют.
 */
const TENSION = 0.75;

export type Shape = {
  /** Путь ломаной. */
  line: string;
  /** Он же, замкнутый по нижнему краю, — заливка под линией. */
  area: string;
};

type Bounds = {
  low: number;
  high: number;
  /** Сколько отсчётов укладывается во всю ширину. */
  capacity: number;
  height: number;
  /** Сглаживать углы кривой вместо прямых отрезков. */
  smooth?: boolean;
};

type Point = { x: number; y: number };

/**
 * Ломаная по значениям от старых к новым.
 *
 * Координаты условные: наружу картинка растягивается `preserveAspectRatio`,
 * поэтому штрих приходится спасать `vectorEffect`. Меньше двух точек — рисовать
 * нечего: линии из одной точки не бывает.
 */
export function shapeOf(values: readonly number[], bounds: Bounds): Shape | null {
  if (values.length < 2) return null;

  const { low, high, capacity, height, smooth = false } = bounds;
  const span = high - low || 1;

  const step = VIEW_WIDTH / Math.max(capacity - 1, 1);
  // Свежая точка всегда у правого края, история уходит влево
  const startX = VIEW_WIDTH - (values.length - 1) * step;

  const points: Point[] = values.map((value, index) => {
    const clamped = Math.min(Math.max(value, low), high);
    return {
      x: startX + index * step,
      y: height - ((clamped - low) / span) * height,
    };
  });

  const line = smooth ? curve(points) : `M ${points.map(at).join(" L ")}`;

  return {
    line,
    area: `${line} L ${VIEW_WIDTH},${height} L ${startX.toFixed(2)},${height} Z`,
  };
}

function at(point: Point): string {
  return `${point.x.toFixed(2)},${point.y.toFixed(2)}`;
}

/**
 * Сжимает ряд до нужного числа корзин, усредняя внутри каждой.
 *
 * Нужен столбикам: отсчётов в истории сто двадцать, столбиков — три десятка,
 * и рисовать по столбику на отсчёт значило бы рисовать штриховку.
 *
 * Усредняем, а не берём наибольшее: загрузка скачет каждую секунду, и по
 * пикам ряд превратился бы в частокол одинаковой высоты. Границы корзин
 * пропорциональные, поэтому крайние не оказываются короче прочих.
 *
 * Пока отсчётов меньше, чем корзин, ряд отдаётся как есть — свободные места
 * достаются вызывающему, и слева остаётся пусто, как у ломаной.
 */
export function bucket(values: readonly number[], count: number): number[] {
  if (count < 1) return [];
  if (values.length <= count) return [...values];

  const buckets: number[] = [];

  for (let index = 0; index < count; index += 1) {
    const from = Math.floor((index * values.length) / count);
    const to = Math.floor(((index + 1) * values.length) / count);

    let sum = 0;
    for (let at = from; at < to; at += 1) sum += values[at]!;
    buckets.push(sum / (to - from));
  }

  return buckets;
}

/**
 * Кубическая кривая через все точки — Catmull-Rom, переписанный в кривые Безье.
 *
 * Направление в каждой точке берётся по её соседям, поэтому кривая проходит
 * ровно через отсчёты, а не мимо них: сглаживание не должно придумывать
 * значения, которых не измеряли. У краёв соседа нет, и точка служит соседом
 * сама себе — так конец не заносит в сторону.
 */
function curve(points: Point[]): string {
  let path = `M ${at(points[0]!)}`;

  for (let index = 0; index < points.length - 1; index += 1) {
    const before = points[index - 1] ?? points[index]!;
    const start = points[index]!;
    const end = points[index + 1]!;
    const after = points[index + 2] ?? end;

    const first = {
      x: start.x + ((end.x - before.x) / 6) * TENSION,
      y: start.y + ((end.y - before.y) / 6) * TENSION,
    };
    const second = {
      x: end.x - ((after.x - start.x) / 6) * TENSION,
      y: end.y - ((after.y - start.y) / 6) * TENSION,
    };

    path += ` C ${at(first)} ${at(second)} ${at(end)}`;
  }

  return path;
}
