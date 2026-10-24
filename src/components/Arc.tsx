import type { CSSProperties, ReactNode } from "react";
import "./Arc.css";

type Props = {
  /** Доля от 0 до 1. `null` — рисуем только трек. */
  value: number | null;
  /**
   * Угол сектора в градусах, отсчёт симметрично от верха.
   * 360 — сплошное кольцо (загрузки у железа), около 220 — шкала (спидтест).
   */
  span?: number;
  /**
   * Размер по внешней стороне, в точках.
   *
   * Обычно не задаётся: кольцо берёт `--arc-size` из токенов, а тот умножен
   * на кегль. Число тут — для витрины, где кольца стоят рядом нарочно разного
   * размера.
   */
  size?: number;
  /**
   * Толщина дуги в единицах `viewBox`, то есть в сотых долях размера.
   *
   * Не в точках: обводка живёт внутри квадрата 100×100, который растягивается
   * наружу. Оттого толщина пропорциональна кольцу сама, и множить её на кегль
   * отдельно не нужно.
   */
  thickness?: number;
  /** Содержимое по центру. */
  children?: ReactNode;
};

/** Внутренняя система координат — квадрат 100×100, наружу масштабируется. */
const VIEW = 100;
const CENTER = VIEW / 2;

function polar(radius: number, degrees: number) {
  // Ноль градусов — верх, дальше по часовой стрелке
  const radians = ((degrees - 90) * Math.PI) / 180;
  return {
    x: CENTER + radius * Math.cos(radians),
    y: CENTER + radius * Math.sin(radians),
  };
}

function arcPath(radius: number, startDegrees: number, endDegrees: number): string {
  const start = polar(radius, startDegrees);
  const end = polar(radius, endDegrees);
  const largeArc = endDegrees - startDegrees > 180 ? 1 : 0;
  return `M ${start.x.toFixed(3)} ${start.y.toFixed(3)} A ${radius} ${radius} 0 ${largeArc} 1 ${end.x.toFixed(3)} ${end.y.toFixed(3)}`;
}

export function Arc({ value, span = 360, size, thickness = 8, children }: Props) {
  const radius = CENTER - thickness / 2;
  const clamped = value === null ? null : Math.min(Math.max(value, 0), 1);
  // pathLength делает длину дуги равной ста единицам независимо от геометрии,
  // так что доля заполнения ложится в strokeDasharray напрямую
  // Шаг — полпроцента: дрожь загрузки в сотых долях иначе каждый такт будила бы
  // переход штриха, а с ним и тень дуги у тем референса, во всех кольцах разом
  const dash = clamped === null ? undefined : `${Math.round(clamped * 200) / 2} 100`;

  const closed = span >= 360;
  // У замкнутого кольца отсчёт штриха начинается справа — доворачиваем к верху
  const shared = {
    pathLength: 100,
    strokeWidth: thickness,
    ...(closed ? { transform: `rotate(-90 ${CENTER} ${CENTER})` } : {}),
  };

  const geometry = closed
    ? { cx: CENTER, cy: CENTER, r: radius }
    : { d: arcPath(radius, -span / 2, span / 2) };

  return (
    <div
      className="arc"
      style={size === undefined ? undefined : ({ "--arc-size": `${size}px` } as CSSProperties)}
    >
      <svg className="arc__svg" viewBox={`0 0 ${VIEW} ${VIEW}`} aria-hidden="true">
        {closed ? (
          <>
            <circle className="arc__track" {...geometry} {...shared} />
            {dash !== undefined && (
              <circle className="arc__value" {...geometry} {...shared} strokeDasharray={dash} />
            )}
          </>
        ) : (
          <>
            <path className="arc__track" {...geometry} {...shared} />
            {dash !== undefined && (
              <path className="arc__value" {...geometry} {...shared} strokeDasharray={dash} />
            )}
          </>
        )}
      </svg>
      {children !== undefined && <div className="arc__center">{children}</div>}
    </div>
  );
}
