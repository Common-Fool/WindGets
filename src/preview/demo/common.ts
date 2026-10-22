import type { Place } from "../../lib/settings";
import type { PreviewState } from "../protocol";

/** Что ответ знает о вопросе: чья рамка, что прислало окно настроек. */
export type Context = {
  id: string;
  state: PreviewState;
  /** Событие в карточку — как от Rust. */
  emit: (event: string, payload: unknown) => void;
};

export type Answer = (args: Record<string, unknown>, context: Context) => unknown;

/** Ответы по имени команды Rust. */
export type Answers = Record<string, Answer>;

/** Город превью погоды — нейтральный пример из правил проекта. */
export const DEMO_PLACE: Place = {
  name: "Graz",
  region: "Steiermark, Österreich",
  latitude: 47.0707,
  longitude: 15.4395,
};

/** Миг «сейчас» превью: от него отсчитываются истории и времена писем. */
export const NOW = Date.now();

export const MINUTE = 60_000;
export const HOUR = 60 * MINUTE;
export const DAY = 24 * HOUR;
export const GIB = 1024 ** 3;
