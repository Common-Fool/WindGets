/*
 * Кнопки пульта — общие на всё приложение: пуск, пауза, соседний трек.
 *
 * Жили в `src/widgets/spotify/icons.tsx`, пока их не понадобилось поставить и
 * в медиа-виджет: жест там тот же самый, и рисовать его вторыми глифами
 * значило бы завести две правды об одном и том же. Переехали сюда, а не
 * импортируются из чужого виджета — виджеты друг о друге не знают. Тот же
 * путь, которым прошли стрелки в `Arrows.tsx`.
 *
 * Фирменные глифы Спотифая (перемешивание, повтор, громкость, плейлисты)
 * остались у него: второго хозяина у них нет.
 *
 * Рисуем сами: ради четырёх глифов тянуть шрифт незачем, а так они наследуют
 * цвет кнопки и растут вместе с кеглем.
 */

import type { ReactNode } from "react";

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true" fill="currentColor">
      {children}
    </svg>
  );
}

export function PlayIcon() {
  return (
    <Glyph>
      <path d="M8 5.5v13a1 1 0 0 0 1.53.85l10-6.5a1 1 0 0 0 0-1.7l-10-6.5A1 1 0 0 0 8 5.5Z" />
    </Glyph>
  );
}

/** Стоп: у прослушивания обрезки в конвертере паузы нет — есть «хватит». */
export function StopIcon() {
  return (
    <Glyph>
      <rect x="6.5" y="6.5" width="11" height="11" rx="2" />
    </Glyph>
  );
}

export function PauseIcon() {
  return (
    <Glyph>
      <rect x="6.5" y="5" width="4" height="14" rx="1.2" />
      <rect x="13.5" y="5" width="4" height="14" rx="1.2" />
    </Glyph>
  );
}

export function PreviousIcon() {
  return (
    <Glyph>
      <rect x="5" y="5.5" width="2.4" height="13" rx="1.2" />
      <path d="M19 6.6v10.8a1 1 0 0 1-1.53.85l-8.4-5.4a1 1 0 0 1 0-1.7l8.4-5.4A1 1 0 0 1 19 6.6Z" />
    </Glyph>
  );
}

export function NextIcon() {
  return (
    <Glyph>
      <path d="M5 6.6v10.8a1 1 0 0 0 1.53.85l8.4-5.4a1 1 0 0 0 0-1.7l-8.4-5.4A1 1 0 0 0 5 6.6Z" />
      <rect x="16.6" y="5.5" width="2.4" height="13" rx="1.2" />
    </Glyph>
  );
}
