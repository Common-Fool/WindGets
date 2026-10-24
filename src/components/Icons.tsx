/*
 * Значки органов управления: копирование, очистка, ответы кнопок, страницы.
 *
 * Общие на все виджеты — сюда они и переехали, когда одни и те же лист поверх
 * листа и корзина понадобились и заметкам, и списку дел. Значки погоды остались
 * у себя: они говорят о погоде, а не о действии, и за пределами той карточки не
 * встречаются.
 *
 * Готовые глифы шрифта (⧉, ⌫, ✓) тут не годятся: рисует их шрифт как захочет,
 * толщина у них своя, а рядом с тонкими линиями карточки они выглядят
 * случайными. Свои контуры растут вместе с кеглем, берут цвет строки и стоят в
 * одной толщине штриха со всеми прочими значками приложения.
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
      strokeWidth={1.6}
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}

/**
 * Круговая стрелка — обновить сейчас, не дожидаясь шага опроса.
 *
 * Действие, а не картинка о предмете карточки: цвета своего не имеет, берёт его
 * у кнопки, в которой стоит, и вместе с ней приглушается. По ней живут погода и
 * почта, и копий у неё быть не должно — разъехавшиеся стрелки в двух карточках
 * заметны сразу.
 *
 * Разомкнутое кольцо с наконечником, а не две стрелки по кругу: на четырнадцати
 * точках вторая превращается в кляксу.
 */
export function RefreshIcon() {
  return (
    <Glyph>
      <path d="M20 12a8 8 0 1 1-2.7-6" />
      <path d="M20.4 4.6v4.6h-4.6" />
    </Glyph>
  );
}

/**
 * Заполненный кружок — отбор непрочитанного.
 *
 * Тот же знак, что стоит слева у непрочитанного письма: кнопка отбирает ровно
 * то, что им помечено, и говорить об этом другим значком было бы враньём.
 */
export function DotIcon() {
  return (
    <svg viewBox="0 0 24 24" width="1em" height="1em" aria-hidden="true">
      <circle cx="12" cy="12" r="5" fill="currentColor" />
    </svg>
  );
}

/** Скрепка — вложение письма. */
export function ClipIcon() {
  return (
    <Glyph>
      <path d="M18.5 11.5 12 18a4.2 4.2 0 0 1-6-6l7-7a2.8 2.8 0 0 1 4 4l-7 7a1.4 1.4 0 0 1-2-2l6.5-6.5" />
    </Glyph>
  );
}

/** Копирование: лист поверх листа. */
export function CopyIcon() {
  return (
    <Glyph>
      <rect x="9" y="9" width="11" height="11" rx="2.5" />
      <path d="M15 6.5V6.5A2.5 2.5 0 0 0 12.5 4H6.5A2.5 2.5 0 0 0 4 6.5v6A2.5 2.5 0 0 0 6.5 15" />
    </Glyph>
  );
}

/**
 * Очистка: корзина.
 *
 * Не крестик и не «стереть влево»: крестик в карточке уже значит неудачу
 * копирования, а стрелка возврата обещала бы отмену, которой у кнопки нет.
 */
export function EraseIcon() {
  return (
    <Glyph>
      <path d="M4.5 7h15" />
      <path d="M9.75 4.5h4.5" />
      <path d="M6.75 7l.7 11.1a2 2 0 0 0 2 1.9h5.1a2 2 0 0 0 2-1.9L17.25 7" />
      <path d="M10.5 10.75v5.5" />
      <path d="M13.5 10.75v5.5" />
    </Glyph>
  );
}

/** Получилось. */
export function DoneIcon() {
  return (
    <Glyph>
      <path d="M5 12.5l4.5 4.5L19 7" />
    </Glyph>
  );
}

/** Не получилось. */
export function FailedIcon() {
  return (
    <Glyph>
      <path d="M6.5 6.5l11 11" />
      <path d="M17.5 6.5l-11 11" />
    </Glyph>
  );
}

/**
 * Убрать сделанное: галочка.
 *
 * Рядом с корзиной, и силуэты у них разные настолько, что не спутать. Галочка
 * тут та же, что стоит в клетках сделанных дел, — кнопка убирает ровно их.
 */
export function CheckIcon() {
  return (
    <Glyph>
      <path d="M5 12.5l4.5 4.5L19 7" />
    </Glyph>
  );
}

/** Ещё одна страница. */
export function PlusIcon() {
  return (
    <Glyph>
      <path d="M12 5.5v13" />
      <path d="M5.5 12h13" />
    </Glyph>
  );
}

/** Пустая клетка дела. */
export function BoxIcon() {
  return (
    <Glyph>
      <rect x="4.5" y="4.5" width="15" height="15" rx="3.5" />
    </Glyph>
  );
}

/** Клетка сделанного дела. */
export function BoxCheckedIcon() {
  return (
    <Glyph>
      <rect x="4.5" y="4.5" width="15" height="15" rx="3.5" />
      <path d="M8.5 12.5l2.5 2.5 5-5" />
    </Glyph>
  );
}

/** Две галочки: пометить всё прочитанным. */
export function CheckAllIcon() {
  return (
    <Glyph>
      <path d="M18 7L8.5 16.5 4 12" />
      <path d="M22 10l-7.5 7.5-2-2" />
    </Glyph>
  );
}

/** Внешняя ссылка / открыть в браузере. */
export function ExternalLinkIcon() {
  return (
    <Glyph>
      <path d="M15 4h5v5" />
      <path d="M10 14L20 4" />
      <path d="M18 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h5" />
    </Glyph>
  );
}

/** Папка: открыть, куда конвертер и загрузчик кладут готовое. */
export function FolderIcon() {
  return (
    <Glyph>
      <path d="M3.5 7.5a2 2 0 0 1 2-2h3.8l2 2.2h7.2a2 2 0 0 1 2 2v7.8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
    </Glyph>
  );
}

/** История: циферблат со стрелкой назад — скачанное за этот запуск. */
export function HistoryIcon() {
  return (
    <Glyph>
      <path d="M4 12a8 8 0 1 0 8-8 8.7 8.7 0 0 0-6 2.4L4 8.5" />
      <path d="M4 4v4.5h4.5" />
      <path d="M12 8v4l3 1.75" />
    </Glyph>
  );
}

/** Крестик: закрыть окно или карточку. */
export function CloseIcon() {
  return (
    <Glyph>
      <path d="M6 6l12 12" />
      <path d="M18 6L6 18" />
    </Glyph>
  );
}

/** Предметные значки: тот же штрих, что у органов управления. */
export function CpuIcon() {
  return (
    <Glyph>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
      <path d="M9 3v3m6-3v3M9 18v3m6-3v3M3 9h3m-3 6h3m12-6h3m-3 6h3" />
    </Glyph>
  );
}

export function GpuIcon() {
  return (
    <Glyph>
      <rect x="3" y="5" width="18" height="13" rx="2" />
      <circle cx="12" cy="11.5" r="4" />
      <path d="m12 7.5 1 3m3 1-3 1m-1 3-1-3m-3-1 3-1M6 18v3m4-3v2m4-2v2" />
    </Glyph>
  );
}

export function MemoryIcon() {
  return (
    <Glyph>
      <rect x="3" y="6" width="18" height="11" rx="2" />
      <path d="M7 10v3m5-3v3m5-3v3M6 17v3m4-3v3m4-3v3m4-3v3" />
    </Glyph>
  );
}

export function DriveIcon() {
  return (
    <Glyph>
      <path d="m5 5-3 10v4a1 1 0 0 0 1 1h18a1 1 0 0 0 1-1v-4L19 5Z" />
      <path d="M2 15h20M6 17.5h.01m3 0h.01" />
    </Glyph>
  );
}

export function TemperatureIcon() {
  return (
    <Glyph>
      <path d="M9 14V5a3 3 0 0 1 6 0v9a5 5 0 1 1-6 0Z" />
      <path d="M12 8v9m5-10h3m-3 4h2" />
      <circle cx="12" cy="18" r="1" />
    </Glyph>
  );
}

export function FanIcon() {
  return (
    <Glyph>
      <circle cx="12" cy="12" r="2" />
      <path d="M10 10C5 7 9 1 13 3c2 1 2 4 0 7m1 0c3-5 9-1 7 3-1 2-4 2-7 0m0 1c5 3 1 9-3 7-2-1-2-4 0-7m-1 0c-3 5-9 1-7-3 1-2 4-2 7 0" />
    </Glyph>
  );
}

export function ClockIcon() {
  return (
    <Glyph>
      <path d="M2 12h4l3-7 5 14 3-7h5" />
    </Glyph>
  );
}

export function PowerIcon() {
  return (
    <Glyph>
      <path d="m13 2-9 12h7l-1 8 10-13h-7Z" />
    </Glyph>
  );
}

export function ShieldIcon() {
  return (
    <Glyph>
      <path d="m12 3 8 3v6c0 5-8 9-8 9s-8-4-8-9V6Z" />
      <path d="m8 12 3 3 5-6" />
    </Glyph>
  );
}

/*
 * Значки колонки окна настроек: у каждого раздела и у каждого виджета в
 * дереве свой (этап 52, по образцу Themia). Те же правила, что у всех
 * значков здесь: поле 24×24, штрих 1.6, цвет строки. Своих логотипов у служб
 * нет намеренно: у Спотифая нота, а не знак бренда, — чужой товарный знак в
 * нашем интерфейсе не нужен.
 */

/** Общие: два ползунка. */
export function SlidersIcon() {
  return (
    <Glyph>
      <path d="M4 7h9m4 0h3M4 17h3m4 0h9" />
      <circle cx="15" cy="7" r="2" />
      <circle cx="9" cy="17" r="2" />
    </Glyph>
  );
}

/** Оформление: палитра с красками. */
export function PaletteIcon() {
  return (
    <Glyph>
      <path d="M12 3.5a8.5 8.5 0 1 0 0 17c1.1 0 1.8-.8 1.8-1.7 0-.5-.2-.9-.5-1.2-.3-.4-.5-.8-.5-1.2 0-1 .8-1.7 1.8-1.7h2a3.9 3.9 0 0 0 3.9-3.9C20.5 7 16.7 3.5 12 3.5Z" />
      <circle cx="7.5" cy="11.5" r="1" />
      <circle cx="10" cy="7.5" r="1" />
      <circle cx="14.5" cy="7.5" r="1" />
    </Glyph>
  );
}

/** Виджеты: четыре плитки. */
export function TilesIcon() {
  return (
    <Glyph>
      <rect x="4" y="4" width="7" height="7" rx="1.5" />
      <rect x="13" y="4" width="7" height="7" rx="1.5" />
      <rect x="4" y="13" width="7" height="7" rx="1.5" />
      <rect x="13" y="13" width="7" height="7" rx="1.5" />
    </Glyph>
  );
}

/** Данные: цилиндр хранилища — откуда приходят показания. */
export function DataIcon() {
  return (
    <Glyph>
      <ellipse cx="12" cy="6" rx="7.5" ry="2.5" />
      <path d="M4.5 6v12c0 1.4 3.4 2.5 7.5 2.5s7.5-1.1 7.5-2.5V6" />
      <path d="M4.5 12c0 1.4 3.4 2.5 7.5 2.5s7.5-1.1 7.5-2.5" />
    </Glyph>
  );
}

/** Лицензия: документ с загнутым углом. */
export function DocumentIcon() {
  return (
    <Glyph>
      <path d="M7 3.5h7l4 4V19a1.5 1.5 0 0 1-1.5 1.5h-9.5A1.5 1.5 0 0 1 5.5 19V5A1.5 1.5 0 0 1 7 3.5Z" />
      <path d="M14 3.5v4h4M8.5 12h7m-7 3.5h7" />
    </Glyph>
  );
}

/** Поддержать: сердце. */
export function HeartIcon() {
  return (
    <Glyph>
      <path d="M12 19.5s-7.5-4.6-7.5-10.2A4.2 4.2 0 0 1 12 6.8a4.2 4.2 0 0 1 7.5 2.5c0 5.6-7.5 10.2-7.5 10.2Z" />
    </Glyph>
  );
}

/** Стрелка раскрытия ветки в колонке: смотрит вниз, повёрнутая CSS — вправо. */
export function ChevronIcon() {
  return (
    <Glyph>
      <path d="m7 10 5 5 5-5" />
    </Glyph>
  );
}

/** Сеть: приём и отдача. */
export function NetworkIcon() {
  return (
    <Glyph>
      <path d="M8 4v16m0 0-3-3m3 3 3-3M16 20V4m0 0-3 3m3-3 3 3" />
    </Glyph>
  );
}

/** Процессы: строки списка с метками слева. */
export function ListIcon() {
  return (
    <Glyph>
      <path d="M5 6.5h1.5M10 6.5h9M5 12h1.5m3.5 0h9M5 17.5h1.5m3.5 0h9" />
    </Glyph>
  );
}

/** Погода: туча, из-за которой выглядывает солнце. */
export function CloudSunIcon() {
  return (
    <Glyph>
      <path d="M8.5 9.5a3.5 3.5 0 0 1 6.3-2.1M8 3.5v1M3.5 8h1M4.8 4.8l.7.7" />
      <path d="M8.5 20h8a3.5 3.5 0 1 0-.8-6.9 5 5 0 0 0-9.5 1.4A2.8 2.8 0 0 0 8.5 20Z" />
    </Glyph>
  );
}

/** Спидтест: шкала со стрелкой. */
export function GaugeIcon() {
  return (
    <Glyph>
      <path d="M4 17a8 8 0 1 1 16 0" />
      <path d="m12 17 4-5.5" />
      <circle cx="12" cy="17" r="1" />
    </Glyph>
  );
}

/** Счётчик кадров: экран и ломаная времени кадра на нём. */
export function FramesIcon() {
  return (
    <Glyph>
      <rect x="3" y="4" width="18" height="13" rx="2" />
      <path d="m6.5 13 3-4 3 2.5 4-5" />
      <path d="M9 20.5h6" />
    </Glyph>
  );
}

/** Звук: динамик с волной. */
export function SoundIcon() {
  return (
    <Glyph>
      <path d="M4.5 9.5h3l4.5-4v13l-4.5-4h-3Z" />
      <path d="M15.5 9a4 4 0 0 1 0 6m2.5-8.5a7.5 7.5 0 0 1 0 11" />
    </Glyph>
  );
}

/** Буфер обмена: планшет с зажимом. */
export function ClipboardIcon() {
  return (
    <Glyph>
      <path d="M9 5.5H7A1.5 1.5 0 0 0 5.5 7v12A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5V7A1.5 1.5 0 0 0 17 5.5h-2" />
      <rect x="9" y="3.5" width="6" height="3.5" rx="1" />
    </Glyph>
  );
}

/** Заметки: блокнот на пружине. */
export function NotepadIcon() {
  return (
    <Glyph>
      <rect x="5.5" y="4.5" width="13" height="16" rx="1.5" />
      <path d="M9 3v3m6-3v3M8.5 11h7m-7 3.5h7m-7 3.5h4" />
    </Glyph>
  );
}

/** Дела: строки с галочками. */
export function ChecklistIcon() {
  return (
    <Glyph>
      <path d="m4 6.5 1.5 1.5L8 5.5M11 7h9M4 13l1.5 1.5L8 12m3 1.5h9M11 19h9M5 19h2" />
    </Glyph>
  );
}

/** Почта: конверт. */
export function EnvelopeIcon() {
  return (
    <Glyph>
      <rect x="3.5" y="5.5" width="17" height="13" rx="2" />
      <path d="m4 7 8 6 8-6" />
    </Glyph>
  );
}

/** Генератор паролей: ключ. */
export function KeyIcon() {
  return (
    <Glyph>
      <circle cx="8" cy="15" r="4" />
      <path d="m11 12 8-8m-3 3 2 2m-4 0 1.5 1.5" />
    </Glyph>
  );
}

/** Коды 2FA: замок. */
export function LockIcon() {
  return (
    <Glyph>
      <rect x="5" y="10.5" width="14" height="10" rx="2" />
      <path d="M8 10.5V8a4 4 0 0 1 8 0v2.5M12 14.5v2" />
    </Glyph>
  );
}

/** Спотифай: нота. Не знак бренда — его товарный знак нам не нужен. */
export function MusicIcon() {
  return (
    <Glyph>
      <path d="M9 17.5V5.5l10-2v12" />
      <circle cx="6.5" cy="17.5" r="2.5" />
      <circle cx="16.5" cy="15.5" r="2.5" />
    </Glyph>
  );
}

/** Медиа: кнопка «играть» в рамке. */
export function MediaIcon() {
  return (
    <Glyph>
      <rect x="3.5" y="5" width="17" height="14" rx="3" />
      <path d="M10.5 9.5v5l4-2.5Z" />
    </Glyph>
  );
}

/** Конвертер: встречные стрелки. */
export function ConvertIcon() {
  return (
    <Glyph>
      <path d="M4 8h13m-3-3 3 3-3 3M20 16H7m3-3-3 3 3 3" />
    </Glyph>
  );
}

/** Загрузчик: стрелка вниз на полку. */
export function DownloadIcon() {
  return (
    <Glyph>
      <path d="M12 4v11m-5-5 5 5 5-5M5 20h14" />
    </Glyph>
  );
}

/** Отладка: жук. */
export function BugIcon() {
  return (
    <Glyph>
      <path d="M8 10a4 4 0 0 1 8 0v5a4 4 0 0 1-8 0Z" />
      <path d="M12 11v8M4.5 11H8m8 0h3.5M4.5 16H8m8 0h3.5M9.5 6.5 8 4.5m6.5 2L16 4.5" />
    </Glyph>
  );
}

/** Витрина компонентов: круг, квадрат и треугольник. */
export function ShapesIcon() {
  return (
    <Glyph>
      <circle cx="7.5" cy="7.5" r="3.5" />
      <rect x="13" y="4" width="7" height="7" rx="1" />
      <path d="m7.5 13.5 4 6.5h-8Z" />
      <circle cx="16.5" cy="16.5" r="3.5" />
    </Glyph>
  );
}

/** Перо — индикатор разрешённой правки заметки. */
export function PenIcon() {
  return (
    <Glyph>
      <path d="m5 16-1 4 4-1L20 7l-3-3L5 16Zm9.5-9.5 3 3" />
    </Glyph>
  );
}

/**
 * Кнопка «поверх всех окон» в дереве настроек: канцелярская кнопка.
 *
 * Своя, а не стрелка вверх: стрелка читается как «поднять один раз», а
 * кнопка — как «приколоть», то есть держать поверх, пока не открепят.
 */
export function PinIcon() {
  return (
    <Glyph>
      <path d="M9 4h6M10 4v5.5L7 13h10l-3-3.5V4" />
      <path d="M12 13v7" />
    </Glyph>
  );
}

/**
 * Открытый замок — окно не заперто. Пара к `LockIcon`: в дереве настроек
 * состояние видно по самой дужке, а не только по яркости значка.
 */
export function UnlockIcon() {
  return (
    <Glyph>
      <rect x="5" y="10.5" width="14" height="10" rx="2" />
      <path d="M8 10.5V8a4 4 0 0 1 7.6-1.7M12 14.5v2" />
    </Glyph>
  );
}
