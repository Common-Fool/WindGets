import { useEffect, useState, type RefObject } from "react";
import { useHeight } from "../../lib/size";

type RowFit = { room: number; pitch: number; rows: number };

/** Пороги относятся к строке у нижнего края: скрыть при 30 %, открыть при 50 %. */
export function fitTotpRows(room: number, pitch: number, previous: RowFit | null): RowFit | null {
  if (!Number.isFinite(room) || !Number.isFinite(pitch) || room <= 0 || pitch <= 0) return null;
  if (previous?.room === room && previous.pitch === pitch) return previous;

  const capacity = room / pitch;
  const before = previous === null ? capacity : previous.room / previous.pitch;
  // Только погрешность арифметики, не дополнительный запас в пикселях.
  const epsilon = 1e-9;
  let rows = Math.floor(capacity + 0.5 + epsilon);
  if (previous !== null) {
    if (capacity > before + epsilon) rows = Math.max(previous.rows, rows);
    else if (capacity < before - epsilon) {
      rows = Math.min(previous.rows, Math.ceil(capacity - 0.7 - epsilon));
    } else rows = previous.rows;
  }
  return { room, pitch, rows: Math.max(0, rows) };
}

/** Направление меняет только новый замер, а не ежесекундная перерисовка кодов. */
export function useFittedTotpHeight(
  space: RefObject<HTMLElement | null>, sample: RefObject<HTMLElement | null>, count: number,
): number | undefined {
  const room = useHeight(space);
  const contentPitch = useHeight(sample);
  const [fit, setFit] = useState<RowFit | null>(null);

  useEffect(() => {
    // У Classic li имеет рамку: contentRect короче реального шага на её толщину.
    const pitch = sample.current?.getBoundingClientRect().height ?? 0;
    setFit((previous) => fitTotpRows(room, pitch, previous));
  }, [room, contentPitch, count, sample]);

  return fit === null ? undefined : fit.rows * fit.pitch;
}
