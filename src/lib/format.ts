import type { Reading, Unit } from "./telemetry";

/** Отсутствующее значение показываем прочерком, никогда не нулём. */
export const NO_VALUE = "—";

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

function formatBytes(bytes: number): string {
  let value = Math.abs(bytes);
  let step = 0;
  while (value >= 1024 && step < BYTE_UNITS.length - 1) {
    value /= 1024;
    step += 1;
  }
  const digits = step === 0 ? 0 : 1;
  return `${(bytes < 0 ? -value : value).toFixed(digits)} ${BYTE_UNITS[step]}`;
}

function formatHertz(megahertz: number): string {
  return megahertz >= 1000
    ? `${(megahertz / 1000).toFixed(2)} GHz`
    : `${megahertz.toFixed(0)} MHz`;
}

export function formatReading(value: number | null, unit: Unit): string {
  if (value === null) return NO_VALUE;

  switch (unit) {
    case "celsius":
      return `${value.toFixed(1)} °C`;
    case "percent":
      return `${value.toFixed(1)} %`;
    case "watt":
      return `${value.toFixed(1)} W`;
    case "megahertz":
      return formatHertz(value);
    case "volt":
      return `${value.toFixed(3)} V`;
    case "rpm":
      return `${value.toFixed(0)} RPM`;
    case "byte":
      return formatBytes(value);
    case "bytePerSecond":
      return `${formatBytes(value)}/s`;
    case "nanosecond":
      return `${value.toFixed(1)} ns`;
    case "raw":
      return value.toFixed(3);
  }
}

/**
 * Отформатированное показание либо `undefined`, если датчика нет вовсе.
 *
 * Разница важна: `undefined` — «такого датчика на этой машине не бывает»,
 * а прочерк внутри строки — «датчик есть, значения нет».
 */
export function showReading(reading: Reading | undefined): string | undefined {
  return reading ? formatReading(reading.value, reading.unit) : undefined;
}

/**
 * Загрузка целым числом процентов: «42 %».
 *
 * ВАЖНО: **та же точность, что в кольце.** `showReading` даёт проценту знак
 * после запятой, и в виде «полосы» загрузка дрожала бы десятыми каждую секунду,
 * а в виде «кольца» стояла бы целым числом — один и тот же показатель читался
 * бы по-разному от вида к виду.
 */
export function showLoad(reading: Reading | undefined): string | undefined {
  return reading?.value == null ? undefined : `${reading.value.toFixed(0)} %`;
}

/** Два показания через дробь: «2,1 ГБ / 15,9 ГБ». */
export function showPair(
  left: Reading | undefined,
  right: Reading | undefined,
): string | undefined {
  if (!left || !right) return undefined;
  return `${formatReading(left.value, left.unit)} / ${formatReading(right.value, right.unit)}`;
}

/**
 * Показание с поправочным множителем.
 *
 * Нужен для частоты видеопамяти: LHM отдаёт физическую частоту, а привычная
 * по спецификациям вдвое выше.
 */
export function showScaled(
  reading: Reading | undefined,
  factor: number,
): string | undefined {
  if (!reading) return undefined;
  const value = reading.value === null ? null : reading.value * factor;
  return formatReading(value, reading.unit);
}

/** Проценты датчика в долю от нуля до единицы — для полос и дуг. */
export function shareOf(reading: Reading | undefined): number | null {
  return reading?.value != null ? reading.value / 100 : null;
}

/**
 * Только число в гигабайтах, без единицы.
 *
 * Нужен там, где единица стоит отдельной подписью и не должна скакать между
 * МБ и ГБ вслед за величиной, как это делает `formatReading`.
 */
export function gigabytes(bytes: number | null | undefined): string | undefined {
  if (bytes == null) return undefined;
  return (bytes / 1024 ** 3).toFixed(1);
}

/**
 * Занято из всего одной строкой: «12.4 / 31.9».
 *
 * Отдельного датчика «всего» у LHM нет, он складывается из занятого и
 * доступного, — поэтому одного значения тут мало.
 *
 * Единицы в строке нет: она стоит рядом отдельной подписью и меньшим кеглем,
 * как у крупных цифр в остальных карточках. Через дробь, а не «of»: так строка
 * короче на два знака, а в самой тесной карточке это решает.
 */
export function usedOfTotal(
  used: number | null | undefined,
  total: number | null | undefined,
): string | undefined {
  const left = gigabytes(used);
  const right = gigabytes(total);
  return left !== undefined && right !== undefined ? `${left} / ${right}` : undefined;
}

/**
 * Размер вложения: «14 KB», «2.3 MB».
 *
 * Единицы английские и те же, что во всех карточках. Килобайт тут двоичный —
 * так его считает и проводник Windows, рядом с которым эти числа и сверяют.
 */
export function fileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return NO_VALUE;
  if (bytes < 1024) return `${Math.round(bytes)} B`;

  const kb = bytes / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB`;

  const mb = kb / 1024;
  return mb < 10 ? `${mb.toFixed(1)} MB` : `${Math.round(mb)} MB`;
}

/** Скорость скачивания: «850 KB/s», «3.2 MB/s». */
export function transferRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return NO_VALUE;
  return `${fileSize(bytesPerSecond)}/s`;
}

/** Длительность: «0:07», «3:07», «1:02:33». Нечисловое — прочерк. */
export function duration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds) || seconds < 0) return NO_VALUE;

  const total = Math.round(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);

  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/**
 * Подпись времени письма: сегодняшнее — «14:20», вчерашнее и старше — «12 окт.»,
 * «Oct 12», «10月12日»: день с месяцем складывает язык, а не свой список.
 *
 * Секунды приходят в UTC, а дату и границу суток меряем в местном поясе
 * машины: день у человека кончается по его часам, а не по Гринвичу.
 */
export function formatLetterTime(at: number | null | undefined, locale: string): string {
  if (at == null || !Number.isFinite(at) || at <= 0) return NO_VALUE;

  const date = new Date(at * 1000);
  const now = new Date();

  const isToday =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();

  if (isToday) {
    const hours = String(date.getHours()).padStart(2, "0");
    const minutes = String(date.getMinutes()).padStart(2, "0");
    return `${hours}:${minutes}`;
  }

  return new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }).format(date);
}

