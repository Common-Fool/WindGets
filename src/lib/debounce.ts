import { useEffect, useState } from "react";

/*
 * Хук отложенного значения.
 *
 * Отдаёт значение, отставшее от переданного на указанное число миллисекунд.
 * При каждом новом значении таймер сбрасывается, при размонтировании —
 * убирается.
 */
export function useDebounced<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState<T>(value);

  useEffect(() => {
    const timer = setTimeout(() => {
      setDebounced(value);
    }, delay);

    return () => {
      clearTimeout(timer);
    };
  }, [value, delay]);

  return debounced;
}
