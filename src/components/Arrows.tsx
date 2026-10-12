/*
 * Стрелки приёма и отдачи — общие на всё приложение.
 *
 * Жили в `src/widgets/network/icons.tsx`, пока их не понадобилось поставить и
 * в спидтест: там те же две величины, и рисовать их вторыми глифами значило бы
 * завести две правды об одном и том же. Переехали сюда, а не импортируются из
 * чужого виджета — виджеты друг о друге не знают.
 *
 * Рисуем сами: ради двух глифов тянуть шрифт незачем, а так они наследуют цвет
 * ячейки и растут вместе с кеглем.
 */

import type { ReactNode } from "react";

function Glyph({ children }: { children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="1em"
      height="1em"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth={2.2}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

/** Приём: вниз. */
export function DownArrow() {
  return (
    <Glyph>
      <path d="M12 5v14" />
      <path d="M6 13.5 12 19.5l6-6" />
    </Glyph>
  );
}

/** Отдача: вверх. */
export function UpArrow() {
  return (
    <Glyph>
      <path d="M12 19V5" />
      <path d="M6 10.5 12 4.5l6 6" />
    </Glyph>
  );
}
