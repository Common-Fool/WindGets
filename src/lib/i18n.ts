import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/*
 * Язык интерфейса.
 *
 * Каталог один на обе стороны — `src/i18n/<код>.json`: Rust берёт те же файлы
 * для трея, меню и имён из реестра. Окно грузит только свой язык, динамическим
 * `import()`: Vite кладёт каждый файл отдельным куском, и пять словарей в
 * каждом из двух десятков окон держать незачем.
 *
 * Выбранный язык знает Rust и присылает событием `language` — всем окнам
 * одинаково. Подписи показаний (CPU, GPU, Temperature…) и единицы в каталог не
 * входят: они английские во всех языках.
 */

export type Language = "ru" | "uk" | "en" | "es" | "zh";

/** Тег для `Intl` и атрибута `lang`. По `zh-CN` WebView2 выбирает китайский
 *  шрифт — с голым `zh` иероглифы достались бы первому попавшемуся. */
const LOCALES: Record<Language, string> = {
  ru: "ru-RU",
  uk: "uk-UA",
  en: "en-US",
  es: "es-ES",
  zh: "zh-CN",
};

const CATALOGS: Record<Language, () => Promise<Record<string, string>>> = {
  ru: () => import("../i18n/ru.json").then((module) => module.default),
  uk: () => import("../i18n/uk.json").then((module) => module.default),
  en: () => import("../i18n/en.json").then((module) => module.default),
  es: () => import("../i18n/es.json").then((module) => module.default),
  zh: () => import("../i18n/zh.json").then((module) => module.default),
};

/**
 * Ключи каталога. Тип снят с русского файла — он источник, — и это только
 * тип: в сборку файл этим не попадает. Опечатка в ключе ловится `tsc`.
 */
export type Key = Exclude<keyof typeof import("../i18n/ru.json"), "default">;

/** Основа ключа с формами множественного числа: `tasks` у `tasks_one`,
 *  `tasks_few`, `tasks_other`. */
export type CountKey = PluralBase<Key>;

// Через параметр, а не прямо по `Key`: условный тип раскладывается по
// объединению только тогда, и без этого `CountKey` выходил бы `never`
type PluralBase<K> = K extends `${infer Base}_other` ? Base : never;

type Args = Record<string, string | number>;

export type Text = {
  /** Строка каталога с подстановками `{name}`. */
  (key: Key, args?: Args): string;
  /**
   * Строка с числом: форму выбирает `Intl.PluralRules` языка — у русского их
   * три, у китайского одна. Нужной формы нет — берётся `_other`. Само число
   * подставляется в `{count}`.
   */
  count: (key: CountKey, count: number, args?: Args) => string;
  language: Language;
  /** Тег для `Intl.DateTimeFormat` и прочих. */
  locale: string;
};

function isLanguage(code: string): code is Language {
  return code in CATALOGS;
}

function fill(text: string, args?: Args): string {
  if (!args) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in args ? String(args[name]) : whole,
  );
}

function makeText(language: Language, texts: Record<string, string> | null): Text {
  const plural = new Intl.PluralRules(LOCALES[language]);

  const t = ((key: Key, args?: Args) => fill(texts?.[key] ?? key, args)) as Text;

  t.count = (key, count, args) => {
    const form = texts?.[`${key}_${plural.select(count)}`] ?? texts?.[`${key}_other`] ?? key;
    return fill(form, { count, ...args });
  };
  t.language = language;
  t.locale = LOCALES[language];

  return t;
}

type State = {
  /** `null` — словарь ещё не приехал, и окну рисовать нечем. */
  texts: Record<string, string> | null;
  text: Text;
  /** Словарь не загрузился ни на своём языке, ни на английском. */
  failed: boolean;
};

let state: State = { texts: null, text: makeText("ru", null), failed: false };
const listeners = new Set<() => void>();
let started = false;

/** Номер последней загрузки: запоздавший ответ прежнего языка не должен
 *  перебить новый. Смена языка сразу после открытия окна иначе откатывалась
 *  бы назад сама. */
let loads = 0;

async function adopt(code: string): Promise<void> {
  const language: Language = isLanguage(code) ? code : "en";
  const ticket = ++loads;

  let texts: Record<string, string>;
  let shown: Language = language;
  try {
    texts = await CATALOGS[language]();
  } catch {
    // ВАЖНО: **неприехавший словарь не оставляет окно пустым навсегда.**
    // Прежде отказ молча возвращался, `Shell` ждал словаря вечно, и окно
    // стояло прозрачным пятном. Сперва английский, потом — честный отказ
    try {
      texts = await CATALOGS.en();
      shown = "en";
    } catch {
      if (ticket !== loads) return;
      state = { ...state, failed: true };
      for (const listener of listeners) listener();
      return;
    }
  }
  if (ticket !== loads) return;

  document.documentElement.lang = LOCALES[shown];
  state = { texts, text: makeText(shown, texts), failed: false };
  for (const listener of listeners) listener();
}

/**
 * Подхватывает язык при открытии окна. Зовётся один раз на окно.
 *
 * Тот же приём, что у вида в `look.ts`: спрашиваем и подписываемся разом, и
 * ответ на вопрос выбрасывается, если событие успело раньше.
 */
export function initText(): void {
  if (started) return;
  started = true;

  let heard = false;

  invoke<string>("language")
    .then((code) => {
      if (!heard) void adopt(code);
    })
    .catch(() => {
      // Вне Tauri спросить не у кого. Язык тогда берётся из адреса,
      // `?lang=zh`, — ради замеров раскладки на каждом языке в браузере
      void adopt(new URLSearchParams(window.location.search).get("lang") ?? "ru");
    });

  listen<string>("language", (event) => {
    heard = true;
    void adopt(event.payload);
  }).catch(() => {});
}

function subscribe(listener: () => void): () => void {
  initText();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Строки на выбранном языке. Компонент перерисуется, когда язык сменят. */
export function useText(): Text {
  return useSyncExternalStore(subscribe, () => state.text);
}

/** Словарь приехал: до него окну рисовать нечем, кроме ключей. */
export function useTextReady(): boolean {
  return useSyncExternalStore(subscribe, () => state.texts !== null);
}

/** Словарь не загрузился вовсе: окну нечем писать, остаётся кнопка повтора. */
export function useTextFailed(): boolean {
  return useSyncExternalStore(subscribe, () => state.failed);
}

/** Выбор языка из окна настроек. `null` — «как в Windows». */
export function setLanguage(language: Language | null): void {
  invoke("set_language", { language }).catch(() => {});
}
