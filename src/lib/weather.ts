import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";
import type { Accent } from "./accent";

/** Погода за окном. Раскладывает код WMO в это Rust — там же, где тесты. */
export type Sky =
  | "clear"
  | "partlyCloudy"
  | "overcast"
  | "fog"
  | "drizzle"
  | "rain"
  | "sleet"
  | "snow"
  | "thunder";

/** Сутки прогноза. `null` — сервис не посчитал, показываем прочерк. */
export type Day = {
  max: number | null;
  min: number | null;
  /** Вероятность осадков в процентах. */
  precipitation: number | null;
};

/** Час прогноза — строка первой и второй зон раскрытия. */
export type Hour = {
  /** Время в поясе города, уже подписью: «15:00». Режет его Rust. */
  at: string;
  sky: Sky;
  temperature: number | null;
  /** Вероятность осадков в процентах. */
  precipitation: number | null;
  /** Скорость ветра в метрах в секунду. */
  wind: number | null;
  day: boolean;
};

/** День в ряду под основным блоком. */
export type Ahead = {
  /** Дата в поясе города, как пришла: «2026-09-03». Имя дня делает карточка. */
  date: string;
  sky: Sky;
  /** Макс, мин и вероятность осадков за сутки. */
  day: Day;
  /** Тот же день по часам: 04:00, 08:00, 12:00, 16:00 и 20:00. */
  windows: Hour[];
};

export type Report = {
  temperature: number | null;
  sky: Sky;
  /** День на дворе или ночь: у ясного неба от этого зависит значок. */
  day: boolean;
  /** Вероятность осадков в ближайший час, проценты. */
  precipitation: number | null;
  /** Ветер сейчас, метры в секунду. */
  wind: number | null;
  /** Нынешний день по часам: шесть значений шагом в два часа. */
  hours: Hour[];
  /** Пять дней, начиная с завтрашнего. */
  days: Ahead[];
};

export type WeatherState =
  /** Город не выбран — виджет ждёт настроек. */
  | { status: "noPlace" }
  | { status: "loading"; place: string }
  | {
      status: "ready";
      place: string;
      report: Report;
      /** Ответа нет давно: на экране прошлая погода. */
      stale: boolean;
    }
  | { status: "failed"; place: string; reason: string };

/*
 * Подписка одна на окно, как у пинга и телеметрии, и снимается, когда
 * слушателей не осталось (`live.ts`). Там же порядок: сперва подписка, потом
 * снимок — событие, ушедшее между ними, иначе пропадало бы, и карточка стояла
 * бы на «Смотрим в окно…» до следующего захода через четыре часа.
 */
export const useWeather = liveSnapshot<WeatherState>("weather_state", "weather", { status: "noPlace" });

/**
 * Обновить прогноз сейчас, не дожидаясь четырёхчасового шага.
 *
 * Ответом считается следующее событие `weather`: Rust шлёт его после
 * принудительного захода даже тогда, когда погода не изменилась ни на градус —
 * иначе значок обновления крутился бы до самого таймаута.
 */
export function refreshWeather(): Promise<void> {
  return invoke<void>("weather_refresh").catch(() => {
    // Вне Tauri обновлять нечего
  });
}

/**
 * Цвет карточки по погоде за окном.
 *
 * Единственное после шкалы накопителей отступление от правила «цвет закреплён
 * за метрикой»: метрики у погоды нет вовсе, а цвет отвечает на вопрос «что там
 * снаружи» — ровно тот, ради которого на виджет и смотрят.
 *
 * Малооблачно остаётся тёплым: солнце за окном есть, и значок показывает его же.
 * Гроза — единственное явление, о котором стоит знать заранее, и красить её как
 * обычный дождь значит его потерять; тёплое занято ясным, поэтому «плохо».
 * Не «тревога»: в палитре из баз та делит цвет со «средне», то есть с тем же
 * тёплым, что у ясного неба, и гроза засветилась бы солнцем.
 */
export function accentOf(sky: Sky): Accent {
  switch (sky) {
    case "clear":
    case "partlyCloudy":
      return "sky-warm";
    case "drizzle":
    case "rain":
    case "sleet":
    case "snow":
      return "sky-water";
    case "thunder":
      return "danger";
    default:
      return "neutral";
  }
}

/**
 * Градусы целыми: карточка не метеостанция, а десятая доля в крупной цифре
 * шевелилась бы каждый опрос, ничего не сообщая.
 *
 * Минус ноль убираем: `Math.round(-0.4)` даёт `-0`, и в строке это видно.
 */
export function degrees(value: number | null): string | undefined {
  if (value === null) return undefined;
  const rounded = Math.round(value);
  return `${rounded === 0 ? 0 : rounded}°`;
}
