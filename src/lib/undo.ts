import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Сколько ждёт возврат после разрушительного действия.
 *
 * Восьми секунд хватает, чтобы заметить пропажу и передумать, и мало, чтобы
 * кнопка успела стать частью карточки.
 */
export const UNDO_FOR = 8000;

/**
 * Возврат последнего разрушительного действия.
 *
 * Один на заметки и на дела: очистка списка и стирание заметки — одно и то же
 * по сути, и второй копии таймера с забыванием быть не должно.
 *
 * ВАЖНО: **снимок хранит всё состояние целиком, поэтому живёт он ровно до
 * следующего действия.** Возврат поверх новых дел стёр бы их, а заодно
 * перебросил бы на ту страницу, которая была открыта до очистки. Снимает
 * предложение сам виджет — из той единственной двери, через которую проходят
 * все его правки.
 */
export function useUndo<T>(): {
  /** Что вернём. `null` — возвращать нечего. */
  kept: T | null;
  /** Запомнить состояние ДО действия. Зовётся после самого действия. */
  remember: (value: T) => void;
  /** Забыть: срок вышел или случилось что-то новее. */
  forget: () => void;
} {
  const [kept, setKept] = useState<T | null>(null);
  const timer = useRef<number | undefined>(undefined);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const forget = useCallback(() => {
    window.clearTimeout(timer.current);
    setKept(null);
  }, []);

  const remember = useCallback((value: T) => {
    setKept(value);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setKept(null), UNDO_FOR);
  }, []);

  return { kept, remember, forget };
}
