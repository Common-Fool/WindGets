import { useEffect, useRef } from "react";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/**
 * Событие «окну поставили готовый размер».
 *
 * Шлёт его Rust, когда размер выбрали в меню виджета. Слушаем только своё
 * окно: общий target Any получает также события остальных карточек. Слушают двое, и оба по
 * необходимости: хук роста забывает свой долг, а карточки с раскрытыми
 * панелями эти панели закрывают.
 *
 * ВАЖНО: **без этого смена размера ломает окно с раскрытой панелью.** Панель
 * растит окно, и хук роста помнит, на сколько именно, — чтобы вернуть его
 * обратно при закрытии. Готовый размер ставит высоту числом, мимо этого
 * счёта; долг остаётся висеть, и следующее закрытие панели отнимает высоту у
 * уже нового размера. Живьём это выглядело как застрявшая плашка микшера,
 * которую пришлось лечить сбросом раскладки.
 */
export const SIZE_EVENT = "widget-size";

export const RESET_EVENT = "widget-reset";

/** Окно перенесли мышью; шлёт его Rust по концу перетаскивания. */
export const MOVED_EVENT = "widget-moved";

function useWindowEvent(event: string, handler: () => void): void {
  const act = useRef(handler);

  useEffect(() => {
    act.current = handler;
  }, [handler]);

  useEffect(() => {
    // Адрес своего окна: события размера Rust шлёт каждому окну отдельно.
    //
    // ВАЖНО: **вне Tauri `getCurrentWebviewWindow` бросает сразу** — метаданных
    // окна нет. Без `try` замер карточки в обычном браузере падал бы пустой
    // страницей; там размеров никто не ставит, и подписка просто не нужна.
    // Отказ подписки ловим сразу, а не в уборке: окно живёт долго, и
    // непойманный до уборки отказ успел бы лечь в консоль.
    let stop: Promise<UnlistenFn | undefined> | undefined;
    try {
      stop = getCurrentWebviewWindow().listen(event, () => act.current()).catch(() => undefined);
    } catch {
      stop = undefined;
    }
    return () => {
      void stop?.then((off) => off?.()).catch(() => {});
    };
  }, [event]);
}

export function useWindowResized(handler: () => void): void {
  useWindowEvent(SIZE_EVENT, handler);
}

/** Сброшена основа: состояние панели остаётся, её высота применяется заново. */
export function useWindowReset(handler: () => void): void {
  useWindowEvent(RESET_EVENT, handler);
}

/** Окно перенесли мышью — место у края экрана могло смениться. */
export function useWindowMoved(handler: () => void): void {
  useWindowEvent(MOVED_EVENT, handler);
}
