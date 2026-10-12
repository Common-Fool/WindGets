/*
 * Иконки пульта. Рисуем сами: ради пяти глифов тянуть шрифт или библиотеку
 * незачем, а так они наследуют цвет от кнопки и масштабируются вместе с ней.
 *
 * Пуск, пауза и соседние треки отсюда уехали в `src/components/Transport.tsx`:
 * их взял и медиа-виджет. Тут остались глифы, у которых хозяин один.
 */

import { useId, type ReactNode } from "react";

/** `weight` — толщина линии у контурного глифа, в долях сетки 24×24. */
type GlyphProps = { children: ReactNode; stroke?: boolean; weight?: number };

function Glyph({ children, stroke = false, weight = 2 }: GlyphProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      aria-hidden="true"
      fill={stroke ? "none" : "currentColor"}
      stroke={stroke ? "currentColor" : "none"}
      strokeWidth={stroke ? weight : undefined}
      strokeLinecap={stroke ? "round" : undefined}
      strokeLinejoin={stroke ? "round" : undefined}
    >
      {children}
    </svg>
  );
}

export function ShuffleIcon() {
  return (
    <Glyph stroke>
      <path d="M3 7h3.5l3 5m0 0 3 5H16" />
      <path d="M3 17h3.5l3-5" />
      <path d="M13 7h3" />
      <path d="m18.5 4.5 2.5 2.5-2.5 2.5" />
      <path d="m18.5 14.5 2.5 2.5-2.5 2.5" />
    </Glyph>
  );
}

export function RepeatIcon() {
  return (
    <Glyph stroke>
      <path d="M4 12V9a3 3 0 0 1 3-3h11" />
      <path d="m15.5 3.5 3 2.5-3 2.5" />
      <path d="M20 12v3a3 3 0 0 1-3 3H6" />
      <path d="m8.5 20.5-3-2.5 3-2.5" />
    </Glyph>
  );
}

/** Повтор одного трека — та же петля с единицей внутри. */
export function RepeatOneIcon() {
  return (
    <Glyph stroke>
      <path d="M4 12V9a3 3 0 0 1 3-3h11" />
      <path d="m15.5 3.5 3 2.5-3 2.5" />
      <path d="M20 12v3a3 3 0 0 1-3 3H6" />
      <path d="m8.5 20.5-3-2.5 3-2.5" />
      <path d="M11.6 9.6 13 9v4" />
    </Glyph>
  );
}

export function VolumeIcon() {
  return (
    <Glyph>
      <path d="M11 5.2 6.9 8.5H4.2a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1h2.7l4.1 3.3a.8.8 0 0 0 1.3-.63V5.83a.8.8 0 0 0-1.3-.63Z" />
      <path
        d="M15.5 9.2a4 4 0 0 1 0 5.6M18 6.7a7.5 7.5 0 0 1 0 10.6"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
      />
    </Glyph>
  );
}

export function PlaylistIcon() {
  return (
    <Glyph stroke>
      <path d="M4 6h11M4 11h11M4 16h7" />
      <path d="M17.5 19a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z" />
      <path d="M19.5 15V8.5" />
    </Glyph>
  );
}

/**
 * Веб-плеер: глобус, привычный значок «в браузере».
 *
 * Линия тоньше обычной: глобус стоит крупно, на кнопке запуска до пятидесяти
 * точек, и при двойке сетки линии там выходили жирными — сказано с живых окон.
 */
export function GlobeIcon() {
  return (
    <Glyph stroke weight={1.25}>
      <circle cx="12" cy="12" r="9" />
      <path d="M3.5 9h17M3.5 15h17" />
      <path d="M12 3c-2.4 2.6-3.6 5.6-3.6 9s1.2 6.4 3.6 9" />
      <path d="M12 3c2.4 2.6 3.6 5.6 3.6 9s-1.2 6.4-3.6 9" />
    </Glyph>
  );
}

/**
 * Дуги знака Спотифая — эталонный контур из Simple Icons, в сетке 24×24 с
 * кругом во весь квадрат. Отдельно от круга: их вырезают маской.
 */
const SPOTIFY_ARCS =
  "M17.521 17.34c-.24.359-.66.48-1.021.24-2.82-1.74-6.36-2.101-10.561-1.141-.418.122-.779-.179-.899-.539-.12-.421.18-.78.54-.9 4.56-1.021 8.52-.6 11.64 1.32.42.18.479.659.301 1.02zm1.44-3.3c-.301.42-.841.6-1.262.3-3.239-1.98-8.159-2.58-11.939-1.38-.479.12-1.02-.12-1.14-.6-.12-.48.12-1.021.6-1.141C9.6 9.9 15 10.561 18.72 12.84c.361.181.54.78.241 1.2zm.12-3.36C15.24 8.4 8.82 8.16 5.16 9.301c-.6.179-1.2-.181-1.38-.721-.18-.601.18-1.2.72-1.381 4.26-1.26 11.28-1.02 15.721 1.621.539.3.719 1.02.419 1.56-.299.421-1.02.599-1.559.3z";

/**
 * Знак Спотифая: круг цветом кнопки с вырезанными дугами.
 *
 * Дуги вырезаны маской и утолщены обводкой на 0,6 сетки. Эталонные дуги рядом
 * с ярлыком Спотифая на рабочем столе выглядели тонкими — там знак нарисован
 * под мелкий размер, с дугами толще, — а на крупной кнопке запуска это видно
 * сразу; из четырёх толщин, поставленных рядом, 0,6 ближе всех к ярлыку и ещё
 * не слепляет просветы на полосе в 66 точек. Первая редакция знака была
 * пересчитана руками и уводила дуги влево до края круга.
 *
 * Сжатие до радиуса 10 оставляет знаку прежнее место среди глифов. Цвета в
 * маске — не цвета: белое пропускает круг, чёрное вырезает, а красится он
 * `currentColor`, как любой глиф. Имя маски своё у каждого экземпляра — в
 * списке плейлистов знаков десятки, и чужая маска вырезала бы не то.
 */
export function SpotifyMark() {
  const mask = `spotify-mark-${useId().replace(/[^\w-]/g, "")}`;

  return (
    <Glyph>
      <defs>
        <mask id={mask} maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24">
          <circle cx="12" cy="12" r="12" fill="#fff" />
          <path
            d={SPOTIFY_ARCS}
            fill="#000"
            stroke="#000"
            strokeWidth={0.6}
            strokeLinejoin="round"
          />
        </mask>
      </defs>
      <g transform="matrix(0.8333 0 0 0.8333 2 2)">
        <circle cx="12" cy="12" r="12" mask={`url(#${mask})`} />
      </g>
    </Glyph>
  );
}
