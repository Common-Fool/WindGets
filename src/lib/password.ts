import type { Accent } from "./accent";
import type { Key } from "./i18n";

/*
 * Генератор паролей. Данных извне не требует и ни от чего не зависит: вся
 * случайность — из `crypto.getRandomValues`, оценка стойкости считается тут же.
 */

/** Строчные буквы включены всегда: без них алфавит вырождается. */
const LOWER = "abcdefghijklmnopqrstuvwxyz";
const UPPER = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS = "0123456789";
const SYMBOLS = "!@#$%^&*()-_=+[]{};:,.?";

export type Options = {
  length: number;
  upper: boolean;
  digits: boolean;
  symbols: boolean;
};

export const MIN_LENGTH = 6;

/**
 * Потолок длины. Он же верх шкалы стойкости: тридцать символов из всех
 * наборов — это 192 бита, и дальше расти уже некуда, любой перебор бессмыслен
 * задолго до этого.
 */
export const MAX_LENGTH = 30;

/**
 * Случайное число от нуля до `bound` без смещения.
 *
 * Отбраковка вместо остатка от деления: остаток раздаёт первым символам
 * алфавита лишнюю вероятность, и тем заметнее, чем алфавит длиннее. Для
 * пароля это прямая потеря стойкости, а не придирка.
 */
function randomBelow(bound: number): number {
  const limit = 256 - (256 % bound);
  const buffer = new Uint8Array(1);

  for (;;) {
    crypto.getRandomValues(buffer);
    const value = buffer[0]!;
    if (value < limit) return value % bound;
  }
}

function pick(alphabet: string): string {
  return alphabet[randomBelow(alphabet.length)]!;
}

/** Наборы, из которых собирается пароль. Первый всегда строчные буквы. */
function alphabets(options: Options): string[] {
  const chosen = [LOWER];
  if (options.upper) chosen.push(UPPER);
  if (options.digits) chosen.push(DIGITS);
  if (options.symbols) chosen.push(SYMBOLS);
  return chosen;
}

export function generate(options: Options): string {
  const chosen = alphabets(options);
  const all = chosen.join("");

  // По одному символу из каждого набора: иначе «цифры включены» иногда
  // означало бы пароль вовсе без цифр, и проверки на той стороне его
  // отвергали бы через раз
  const characters = chosen.map(pick);
  while (characters.length < options.length) {
    characters.push(pick(all));
  }

  // Перемешивание Фишера — Йетса: без него первые символы всегда шли бы
  // в порядке наборов — буква, заглавная, цифра, знак
  for (let index = characters.length - 1; index > 0; index -= 1) {
    const swap = randomBelow(index + 1);
    [characters[index], characters[swap]] = [characters[swap]!, characters[index]!];
  }

  return characters.join("");
}

/** Разряд символа. По нему пароль и красится. */
export type CharClass = "lower" | "upper" | "digit" | "symbol";

export function classOf(character: string): CharClass {
  if (DIGITS.includes(character)) return "digit";
  if (UPPER.includes(character)) return "upper";
  if (LOWER.includes(character)) return "lower";
  return "symbol";
}

/**
 * Сколько символов каждого разряда получилось.
 *
 * Считаем по готовому паролю, а не по переключателям: показывать надо то, что
 * вышло, — на короткой длине включённый набор может дать один символ, а то и
 * ни одного, если наборов больше, чем места.
 */
export function composition(password: string): Record<CharClass, number> {
  const counts: Record<CharClass, number> = { lower: 0, upper: 0, digit: 0, symbol: 0 };
  for (const character of password) counts[classOf(character)] += 1;
  return counts;
}

export type Strength = {
  /** Сколько сегментов закрашено, от одного до [`STRENGTH_STEPS`]. */
  level: number;
  /** Ключ каталога строк: «слабый», «средний», «хороший», «стойкий». */
  label: Key;
  accent: Accent;
  /** Энтропия в битах. */
  bits: number;
};

/** Ступеней в шкале стойкости. */
export const STRENGTH_STEPS = 10;

/** Энтропия самого слабого допустимого пароля: шесть строчных букв. */
const WEAKEST_BITS = Math.round(MIN_LENGTH * Math.log2(LOWER.length));

/** И самого сильного: тридцать символов из всех наборов. */
const STRONGEST_BITS = Math.round(
  MAX_LENGTH * Math.log2((LOWER + UPPER + DIGITS + SYMBOLS).length),
);

/**
 * Оценка стойкости через энтропию: длина, умноженная на двоичный логарифм
 * алфавита. Это верхняя граница для перебора вслепую — словарные атаки она
 * не учитывает, но случайному паролю они и не грозят.
 *
 * Ступеней десять, а не четыре, и растянуты они ровно между самым слабым и
 * самым сильным паролем, который тут можно собрать. На четырёх ступенях
 * двадцать строчных букв и двенадцать символов всех наборов упирались в верх
 * шкалы вместе, хотя между ними семнадцать бит разницы — длина даёт больше,
 * чем разнообразие алфавита.
 */
export function strengthOf(options: Options): Strength {
  const size = alphabets(options).join("").length;
  const bits = Math.round(options.length * Math.log2(size));

  const span = STRONGEST_BITS - WEAKEST_BITS;
  const reached = ((bits - WEAKEST_BITS) * (STRENGTH_STEPS - 1)) / span;
  const level = Math.min(Math.max(1 + Math.floor(reached), 1), STRENGTH_STEPS);

  if (level <= 2) return { level, label: "password.weak", accent: "danger", bits };
  if (level <= 4) return { level, label: "password.fair", accent: "warn", bits };
  if (level <= 7) return { level, label: "password.good", accent: "ok", bits };
  return { level, label: "password.strong", accent: "ok", bits };
}
