import { useEffect, useState, type PointerEvent } from "react";

/** Дольше этого отпущенное место не держится, даже если ответа не было. */
const HELD_LONGEST_MS = 4000;

type Handlers = {
  onPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  onPointerMove: (event: PointerEvent<HTMLDivElement>) => void;
  onPointerUp: (event: PointerEvent<HTMLDivElement>) => void;
  onPointerCancel: () => void;
};

export type SeekDrag = {
  /** Доля, которую рисовать: под мышью, удержанная после отпускания или настоящая. */
  shown: number;
  /** Выбранная доля, пока полосу тянут или ждут ответа; иначе `null`. */
  chosen: number | null;
  /** Вешаются на саму полосу — ту, что помечена `data-no-drag`. */
  handlers: Handlers;
};

/**
 * Полоса перемотки, которую тянут мышью, как ползунок громкости (11 октября,
 * слово пользователя). Общая у пульта Спотифая и медиа-виджета.
 *
 * Пока кнопка зажата, заливка и время идут за мышью, а перемотка уходит одна —
 * на отпускании: слать её на каждое движение значило бы десятки запросов ради
 * места, которое человек ещё выбирает. Щелчок — та же протяжка длиной в ноль.
 *
 * ВАЖНО: **отпущенное место держится до ответа.** Досчёт позиции идёт от
 * последнего снимка, а свежий приходит через сотни миллисекунд: отпущенная
 * полоса скакала назад, к старому месту, и только потом — на выбранное.
 * Держим, пока досчёт не подойдёт к выбранному ближе `close` (в тех же
 * единицах, что `total`), но не дольше четырёх секунд: не прошла перемотка —
 * полоса вернётся к правде.
 *
 * ВАЖНО: **полоса обязана нести `data-no-drag`.** Иначе протяжка по ней
 * уводила бы окно — перетаскивание ловит любое нажатие мимо `KEEPS_THE_MOUSE`.
 */
export function useSeekDrag(
  ratio: number,
  total: number,
  close: number,
  enabled: boolean,
  onSeek: (share: number) => void,
): SeekDrag {
  const [dragged, setDragged] = useState<number | null>(null);
  const [held, setHeld] = useState<number | null>(null);

  useEffect(() => {
    if (held !== null && Math.abs(ratio - held) * total < close) setHeld(null);
  }, [held, ratio, total, close]);

  // Срок — отдельно: досчёт меняется несколько раз в секунду и сбрасывал бы его
  useEffect(() => {
    if (held === null) return;
    const timer = window.setTimeout(() => setHeld(null), HELD_LONGEST_MS);
    return () => window.clearTimeout(timer);
  }, [held]);

  function shareAt(event: PointerEvent<HTMLDivElement>): number {
    const bounds = event.currentTarget.getBoundingClientRect();
    if (bounds.width <= 0) return 0;
    return Math.min(Math.max((event.clientX - bounds.left) / bounds.width, 0), 1);
  }

  const chosen = dragged ?? held;

  return {
    shown: chosen ?? ratio,
    chosen,
    handlers: {
      onPointerDown: (event) => {
        if (event.button !== 0 || !enabled) return;
        event.currentTarget.setPointerCapture(event.pointerId);
        setDragged(shareAt(event));
      },
      onPointerMove: (event) => {
        if (dragged !== null) setDragged(shareAt(event));
      },
      onPointerUp: (event) => {
        if (dragged === null) return;
        const share = shareAt(event);
        setDragged(null);
        setHeld(share);
        onSeek(share);
      },
      onPointerCancel: () => setDragged(null),
    },
  };
}
