/*
 * Значки погоды. Рисуем сами, как у звука и у пульта: ради девяти глифов тянуть
 * библиотеку незачем, а так они растут вместе с кеглем и берут цвет строки.
 *
 * Ночная пара есть только у ясного неба и малооблачности: под сплошными тучами
 * луны не видно, и рисовать её там значило бы врать.
 *
 * ЦВЕТ. Одноцветными они были ровно до тех пор, пока стояли по одному. Теперь
 * их в карточке семь разом — крупный в основном блоке, пять в ряду дней и по
 * одному в каждой строке раскрытой панели, — и различать их приходится с одного
 * взгляда:
 *
 * - солнце, луна и молния — тёплой базой темы (`--sky-warm`);
 * - дождь, морось, снег и гололёд — холодной (`--sky-water`);
 * - облака, туман и ветер берут цвет строки, в которой стоят, — то есть
 *   приглушены там, где приглушена вся клетка.
 *
 * Заливка есть только у тучи с осадками: `fill` у неё гасит контур под струями
 * и отделяет тучу от лёгкого облачка, которое остаётся пустым.
 */

import type { ReactNode } from "react";
import type { Sky } from "../../lib/weather";

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

/** Тёплое: солнце, луна, молния. */
const WARM = "var(--sky-warm)";

/** Мокрое: дождь, морось, снег, гололёд. */
const WATER = "var(--sky-water)";

/** Облако — общее у семи глифов из девяти. */
const CLOUD = <path d="M7 18.5h9.5a3.75 3.75 0 0 0 .4-7.48 5.5 5.5 0 0 0-10.6-1.1A4.2 4.2 0 0 0 7 18.5Z" />;

/**
 * Облако, поднятое к верху поля: под ним рисуются осадки.
 *
 * Отдельным контуром, а не сдвигом общего: сдвиг пришлось бы повторять в каждом
 * глифе с осадками и однажды разойтись на пиксель.
 */
const CLOUD_HIGH = <path d="M7 14.5h9.5a3.75 3.75 0 0 0 .4-7.48 5.5 5.5 0 0 0-10.6-1.1A4.2 4.2 0 0 0 7 14.5Z" />;

/**
 * Та же туча, но залитая, — под ней всегда что-то идёт.
 *
 * Заливка не украшение: под контурной тучей струи и снежинки читаются как
 * продолжение её контура, а под залитой отделяются от неё сразу. Лёгкое облачко
 * при этом остаётся пустым, и по заливке эти двое различаются с одного взгляда.
 */
const CLOUD_FILLED = (
  <path
    d="M7 14.5h9.5a3.75 3.75 0 0 0 .4-7.48 5.5 5.5 0 0 0-10.6-1.1A4.2 4.2 0 0 0 7 14.5Z"
    fill="var(--sky-cloud-fill)"
  />
);

function SunIcon() {
  return (
    <Glyph>
      <circle cx="12" cy="12" r="4.4" stroke={WARM} />
      {/* Восемь лучей: на шести круг читается как шестерёнка */}
      <path d="M12 2.6v2.4M12 19v2.4M2.6 12h2.4M19 12h2.4" stroke={WARM} />
      <path d="M5.4 5.4 7.1 7.1M16.9 16.9l1.7 1.7M18.6 5.4 16.9 7.1M7.1 16.9l-1.7 1.7" stroke={WARM} />
    </Glyph>
  );
}

function MoonIcon() {
  return (
    <Glyph>
      <path d="M20 14.2A8.2 8.2 0 0 1 9.8 4a8.4 8.4 0 1 0 10.2 10.2Z" stroke={WARM} />
    </Glyph>
  );
}

function SunCloudIcon() {
  return (
    <Glyph>
      {/* Солнце выглядывает из-за левого плеча облака */}
      <circle cx="8.6" cy="7.6" r="3.1" stroke={WARM} />
      <path d="M8.6 1.9v1.4M3.4 7.6H2M13.8 7.6h-1.4M4.9 3.9l1 1M12.3 3.9l-1 1" stroke={WARM} />
      {CLOUD}
    </Glyph>
  );
}

function MoonCloudIcon() {
  return (
    <Glyph>
      <path d="M14.6 3.2a4.6 4.6 0 0 0 4.9 6.9 4.7 4.7 0 0 1-8-3.4 4.7 4.7 0 0 1 3.1-3.5Z" stroke={WARM} />
      {CLOUD}
    </Glyph>
  );
}

function CloudIcon() {
  return <Glyph>{CLOUD}</Glyph>;
}

/*
 * Туман: облако и полосы под ним — всё цветом строки.
 *
 * Синими полосы делать нельзя: туман не осадки, вода в нём висит, а не падает,
 * и синий обещал бы зонт там, где он не нужен.
 */
function FogIcon() {
  return (
    <Glyph>
      {CLOUD_HIGH}
      {/* Три полосы разной длины: ровные читались бы как нотный стан */}
      <path d="M5 17.6h11M8 20.5h10" />
    </Glyph>
  );
}

function DrizzleIcon() {
  return (
    <Glyph>
      {CLOUD_FILLED}
      {CLOUD_HIGH}
      <path d="M9 17.6v1.2M13 17.6v1.2M11 20.4v1.2" stroke={WATER} />
    </Glyph>
  );
}

function RainIcon() {
  return (
    <Glyph>
      {CLOUD_FILLED}
      {CLOUD_HIGH}
      {/* Косые струи: отвесные не отличить от мороси на мелком кегле */}
      <path d="M9.4 17.4 8 21M13 17.4l-1.4 3.6M16.6 17.4 15.2 21" stroke={WATER} />
    </Glyph>
  );
}

/** Шесть лучей из одной точки — снежинка размером в пару пикселей. */
function flake(x: number, y: number, arm: number) {
  const slant = arm * 0.72;
  return (
    <path
      stroke={WATER}
      d={`M${x} ${y - arm}v${arm * 2}M${x - slant} ${y - slant / 2}l${slant * 2} ${slant}M${
        x + slant
      } ${y - slant / 2}l${-slant * 2} ${slant}`}
    />
  );
}

function SnowIcon() {
  return (
    <Glyph>
      {CLOUD_FILLED}
      {CLOUD_HIGH}
      {flake(9, 18.2, 1.7)}
      {flake(15, 18.2, 1.7)}
      {flake(12, 21, 1.7)}
    </Glyph>
  );
}

function SleetIcon() {
  return (
    <Glyph>
      {CLOUD_FILLED}
      {CLOUD_HIGH}
      {/* Капля и снежинка вместе: гололёд — это ровно их встреча */}
      <path d="M9.4 17.4 8 21" stroke={WATER} />
      {flake(14.5, 19.4, 1.9)}
    </Glyph>
  );
}

function ThunderIcon() {
  return (
    <Glyph>
      {CLOUD_FILLED}
      {CLOUD_HIGH}
      {/* Хвост молнии кончается на 22,7: ниже его срезала бы граница поля
          вместе с половиной толщины штриха */}
      <path d="M13.6 16.2 10 19.8h3.1l-1.5 2.9" stroke={WARM} />
    </Glyph>
  );
}

/** Значок по погоде за окном. Ночь спрашивается только у ясного неба. */
export function WeatherIcon({ sky, day }: { sky: Sky; day: boolean }) {
  switch (sky) {
    case "clear":
      return day ? <SunIcon /> : <MoonIcon />;
    case "partlyCloudy":
      return day ? <SunCloudIcon /> : <MoonCloudIcon />;
    case "fog":
      return <FogIcon />;
    case "drizzle":
      return <DrizzleIcon />;
    case "rain":
      return <RainIcon />;
    case "sleet":
      return <SleetIcon />;
    case "snow":
      return <SnowIcon />;
    case "thunder":
      return <ThunderIcon />;
    default:
      return <CloudIcon />;
  }
}

/**
 * Место на карте — значок карточки, пока город не выбран.
 *
 * Не погода: показывать там солнце или тучу значило бы сообщить о небе то,
 * чего мы не знаем. Спрашиваем мы ровно место, его и рисуем.
 */
export function PinIcon() {
  return (
    <Glyph>
      <path d="M12 21.5s7-6.1 7-11a7 7 0 1 0-14 0c0 4.9 7 11 7 11Z" />
      <circle cx="12" cy="10.5" r="2.6" />
    </Glyph>
  );
}
