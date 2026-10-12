import { useRef, type CSSProperties } from "react";
import { bucket } from "../lib/chart";
import { useScale } from "../lib/look";
import { useWidth } from "../lib/size";
import "./Columns.css";

/**
 * Место под столбик с просветом, в точках при кегле 100 %.
 *
 * Уже пяти столбик перестаёт быть столбиком: ряд читается как штриховка, а
 * заливка в нём уже собственного скругления. Множится на геометрию вместе с
 * просветом между столбиками — иначе на крупном кегле просвет съел бы столбик.
 */
const SLOT = 7;

/**
 * Потолок числа столбиков.
 *
 * Он же задаёт шаг столбика: истории пять минут, и на трёх десятках на столбик
 * приходится по десять секунд. Меньше десяти — это уже рябь такта опроса, а не
 * ход загрузки; больше столбиков ряд и не вместит, не превратившись в
 * штриховку.
 */
const MOST = 30;

type Props = {
  /** Значения от старых к новым. */
  values: readonly number[];
  /** Границы шкалы. По умолчанию проценты. */
  min?: number;
  max?: number;
  /**
   * Высота в точках.
   *
   * Обычно не задаётся: диаграмма берёт `--columns-height` из места, где
   * стоит. В строке с кольцом это `--arc-size`, иначе половины строки
   * разъехались бы по высоте.
   */
  height?: number;
};

/**
 * История показателя столбиками.
 *
 * Текущего значения не показывает: оно стоит крупно в кольце слева от неё.
 *
 * Не картинка: `<span>` во флексе с высотой в процентах тянется вместе с
 * окном сам, без `viewBox` и его соотношения сторон. На этом уже обжигались
 * с [`Trend`] — абсолютно поставленная `svg` с `auto` размерами берёт
 * пропорции картинки вместо коробки и остаётся квадратной при любом окне.
 *
 * Число столбиков считается по месту, а не задаётся числом: одна и та же
 * диаграмма стоит в окнах разной ширины и переживает растягивание мышью.
 * История длиннее ряда, поэтому отсчёты усредняются по корзинам
 * ([`bucket`]). Пока их меньше, чем столбиков, слева остаётся пусто: свежий
 * отсчёт всегда у правого края, как и у ломаной.
 */
export function Columns({ values, min = 0, max = 100, height }: Props) {
  const row = useRef<HTMLDivElement>(null);
  const width = useWidth(row);
  const geometry = useScale();

  const count = Math.min(Math.floor(width / (SLOT * geometry)), MOST);
  const span = max - min || 1;

  const filled = bucket(values, count);
  const empty = Math.max(count - filled.length, 0);
  const shares = filled.map((sample) => Math.min(Math.max((sample - min) / span, 0), 1));

  return (
    <div
      className="columns"
      style={
        height === undefined ? undefined : ({ "--columns-height": `${height}px` } as CSSProperties)
      }
      ref={row}
      aria-hidden="true"
    >
      {Array.from({ length: empty }, (_, index) => (
        <span key={`empty-${index}`} className="columns__bar" />
      ))}
      {shares.map((share, index) => (
        <span key={index} className="columns__bar">
          <span className="columns__fill" style={{ height: `${(share * 100).toFixed(1)}%` }} />
        </span>
      ))}
    </div>
  );
}
