import { useCallback, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

/*
 * Вид окна: плотность фона карточки, масштаб, цветовая тема, стекло и яркость
 * цветов. Всё хранится в настройках на стороне Rust, оттуда же приезжает
 * событием и переживает перезапуск.
 *
 * Плотность и масштаб бывают своими у виджета, тема и стекло — общие на всё
 * приложение. Общий дизайн выбирает визуал готовых виджетов целиком.
 */

/** Плотная заливка — вид по умолчанию, тот же, что в токенах. */
export const DEFAULT_OPACITY = 0.94;

/**
 * Ниже фон начинает зависеть от обоев: настоящего размытия под окном нет, и
 * мелкий текст на светлых обоях перестаёт читаться. Совпадает с Rust.
 */
export const MIN_OPACITY = 0.86;

/** Масштаб как в токенах. Пределы тоже совпадают с Rust. */
export const DEFAULT_SCALE = 1;
export const MIN_SCALE = 0.9;
export const MAX_SCALE = 1.3;

/** Тема по умолчанию — та, которой токены рисуют окно без `data-palette`. */
export const DEFAULT_PALETTE = "reference-classic";

/** Яркость цветов как в токенах. Пределы совпадают с Rust. */
export const DEFAULT_VIVID = 1;
export const MIN_VIVID = 0.7;
export const MAX_VIVID = 1.5;

/*
 * Стекло и цвета режимов по умолчанию. Те же числа стоят в `settings.rs` и в
 * tokens.css: окно рисует ими, пока настройки ещё не доехали.
 */
export const DEFAULT_GLASS = "obsidian";
export const DEFAULT_ACCENT_COLOR = "#4CA5FF";
export const DEFAULT_CUSTOM_COLORS: Record<string, string> = {
  accent: "#4CA5FF",
  positive: "#40D9A3",
  caution: "#F0B857",
  negative: "#F17882",
};

export type Design = "app" | "reference";

export type Look = {
  design: Design;
  effectiveDesign: Design;
  cardOpacity: number;
  scale: number;
  /** Цветовая тема: идентификатор блока `[data-palette]` в токенах. */
  palette: string;
  /** Стекло: идентификатор блока `[data-glass]`. От темы не зависит. */
  glass: string;
  /** Цвет режима «Один акцент». */
  accentColor: string;
  /** Четыре базы режима «Своя палитра», ключ — роль. */
  customColors: Record<string, string>;
  /** Яркость цветов: светлота и цветность баз разом. */
  vivid: number;
};

/** Числа, которые двигают ползунками. Тему и стекло выбирают нажатием. */
type Sliders = Pick<Look, "cardOpacity" | "scale" | "vivid">;

let look: Look = {
  design: "reference",
  effectiveDesign: "app",
  cardOpacity: DEFAULT_OPACITY,
  scale: DEFAULT_SCALE,
  palette: DEFAULT_PALETTE,
  glass: DEFAULT_GLASS,
  accentColor: DEFAULT_ACCENT_COLOR,
  customColors: DEFAULT_CUSTOM_COLORS,
  vivid: DEFAULT_VIVID,
};

/*
 * Очередь правок вида.
 *
 * ВАЖНО: **порядок вызовов IPC не гарантирован, и на виде это видно.** Каждый
 * `invoke` Tauri обрабатывает своей задачей, поэтому «показать начерно» и
 * «запомнить» приходят в Rust наперегонки. Обгони `commit` последний `preview`
 * — и он пересчитает окна по ПРЕЖНЕМУ числу, увидит «масштаб не менялся» и не
 * тронет ни одного окна; `preview` доедет следом и подвинет только карточку.
 *
 * Снаружи это выглядит так, будто ползунок масштаба меняет буквы, но не окна, —
 * и надёжнее всего ловится одиночным нажатием по дорожке ползунка, где `preview`
 * ровно один и обогнать его легче всего.
 *
 * Тот же приём, что у роста окна в `grow.ts`, и по той же причине.
 */
let queue: Promise<unknown> = Promise.resolve();

function ordered(command: string, args: Record<string, unknown>): void {
  queue = queue.then(() => invoke(command, args)).catch(() => {});
}

/** Когда уйдут все правки вида, поставленные в очередь до этого вызова. */
export function lookSettled(): Promise<unknown> {
  return queue;
}
const listeners = new Set<() => void>();
let started = false;
let appearanceVersion = 0;

/**
 * Окна настроек живут своим видом: ползунки правят виджеты, а не окно, в
 * котором их двигают. Иначе неудачное значение сделало бы нечитаемым тот
 * самый ползунок, которым его правят.
 *
 * Решается здесь, а не в CSS: переменная, собранная из другой переменной
 * (`--surface-card` из `--surface-card-alpha`, `--text-*` из `--scale`),
 * вычисляется там, где объявлена, — в `:root`. Потомки наследуют уже готовое
 * значение, и переопределить множитель ниже по дереву нельзя.
 */
const FIXED_LOOK =
  (new URLSearchParams(window.location.search).get("widget") ?? "") === "settings";

function sameColors(next: Record<string, string>, now: Record<string, string>): boolean {
  const names = Object.keys(next);
  return (
    names.length === Object.keys(now).length && names.every((name) => next[name] === now[name])
  );
}

function adopt(received: Look) {
  appearanceVersion += 1;
  // Старый backend не знает дизайн: показываем Claude, а не ложный выбор GPT.
  const next: Look = { ...received,
    design: received.design ?? "app", effectiveDesign: received.effectiveDesign ?? "app",
    customColors: received.customColors ?? DEFAULT_CUSTOM_COLORS };
  if (!FIXED_LOOK) {
    // Правим корень документа, а не карточку: подписчиков у переменных много
    const style = document.documentElement.style;
    style.setProperty("--surface-card-alpha", String(next.cardOpacity));
    style.setProperty("--scale", String(next.scale));
  }

  // Тема, стекло и яркость — исключение из FIXED_LOOK: окно настроек обязано
  // сразу показать выбранное в нём же. Фиксированы только плотность и масштаб,
  // чтобы управляющее ими окно не стало нечитаемым
  const root = document.documentElement;
  root.dataset.design = next.effectiveDesign;
  root.dataset.palette = next.palette;
  root.dataset.glass = next.glass;
  root.style.setProperty("--single-accent", next.accentColor);
  for (const [role, fallback] of Object.entries(DEFAULT_CUSTOM_COLORS)) {
    root.style.setProperty(`--custom-${role}`, next.customColors[role] ?? fallback);
  }
  root.style.setProperty("--vivid", String(next.vivid));

  if (
    next.design === look.design &&
    next.effectiveDesign === look.effectiveDesign &&
    next.cardOpacity === look.cardOpacity &&
    next.scale === look.scale &&
    next.palette === look.palette &&
    next.glass === look.glass &&
    next.accentColor === look.accentColor &&
    next.vivid === look.vivid &&
    sameColors(next.customColors, look.customColors)
  ) {
    return;
  }

  look = next;
  for (const listener of listeners) listener();
}

/**
 * Подхватывает вид при открытии окна. Зовётся один раз на окно.
 *
 * ВАЖНО: **ответ на первый вопрос выбрасывается, если событие успело раньше.**
 * Спрашиваем и подписываемся одновременно, и порядок ответов не гарантирован:
 * человек, переключивший тему сразу после открытия окна, получил бы её
 * событием, а следом — старый снимок от запоздавшего `invoke`, и окно откатилось
 * бы назад само по себе. Снаружи это выглядит как «настройка не применилась».
 */
export function initLook(): void {
  if (started) return;
  started = true;

  let heard = false;
  const initialVersion = appearanceVersion;

  invoke<Look>("appearance")
    .then((first) => {
      if (heard || appearanceVersion !== initialVersion) return;
      adopt(first);
    })
    .catch(() => {
      // Вне Tauri спросить не у кого — остаёмся на значениях из токенов
    });

  // emit_to рассылает разные снимки каждому окну. Глобальный listen с
  // target Any ловит также чужие снимки (у настроек effectiveDesign=app)
  // и отменяет только что выбранный GPT. Слушаем лишь свою метку окна.
  try {
    getCurrentWebviewWindow().listen<Look>("appearance", (event) => {
      heard = true;
      adopt(event.payload);
    }).catch(() => {});
  } catch {
    // В браузерном макете нет метаданных окна Tauri.
  }
}

function subscribe(listener: () => void): () => void {
  initLook();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

type LookControl = Look & {
  /**
   * Показать значения во всех окнах. На диск они не идут: ползунок зовёт это
   * десятки раз в секунду.
   */
  preview: (next: Partial<Sliders>) => void;
  /** Запомнить накрученное. Зовётся один раз — когда ползунок отпустили. */
  commit: () => void;
};

/**
 * Масштаб для разметки, которая считает точки сама: столбики и списки.
 *
 * То же число, что у `--scale` в tokens.css. Берётся отсюда, а не из
 * `getComputedStyle`: читать вычисленное значение переменной на каждый кадр
 * дороже, чем держать его в памяти.
 *
 * Множителей было два — кегль и геометрия вполовину его хода; половинный
 * ушёл вместе с ползунком «размер шрифта», см. tokens.css.
 */
export function useScale(): number {
  return useSyncExternalStore(subscribe, () => look).scale;
}

export function useLook(): LookControl {
  const value = useSyncExternalStore(subscribe, () => look);

  const preview = useCallback(
    (next: Partial<Sliders>) => {
      // Своё состояние не трогаем: оно приедет событием, одинаково во все окна.
      // Только тронутое: недостающее Rust возьмёт нынешним, а не тем, что
      // окно видело в последнем событии, — оно могло не доехать
      ordered("preview_appearance", {
        cardOpacity: next.cardOpacity ?? null,
        scale: next.scale ?? null,
        vivid: next.vivid ?? null,
      });
    },
    [],
  );

  return { ...value, preview, commit: commitLook };
}

/**
 * Своя плотность фона одного виджета, начерно.
 *
 * `null` — «как у всех»: виджет возвращается под общий ползунок. Событие
 * уходит только в его окно, поэтому окно, из которого крутят ползунок, своего
 * вида не меняет — и правильно: настраивают не его.
 */
export function previewWidgetOpacity(id: string, cardOpacity: number | null): void {
  ordered("preview_widget_appearance", { id, cardOpacity });
}

/**
 * Свой масштаб одного виджета, начерно. `null` — «как у всех».
 *
 * Размер окна за ним подтянется в `commitLook`, а не сейчас: ползунок зовёт
 * это десятки раз в секунду, и окно прыгало бы под мышью.
 */
export function previewWidgetScale(id: string, scale: number | null): void {
  ordered("preview_widget_scale_appearance", { id, scale });
}

/** Запомнить накрученное — общее и по виджетам разом, оно всё в одном файле. */
export function commitLook(): void {
  ordered("commit_appearance", {});
}

/**
 * Цветовая тема — сразу и начисто.
 *
 * Без пары «начерно — начисто»: её выбирают нажатием, а не тянут ползунком, и
 * промежуточных значений у неё нет вовсе. То же у стекла.
 */
export function setPalette(name: string, design: Design = look.effectiveDesign): void {
  ordered("set_palette", { name, design });
}

export function setGlass(name: string, design: Design = look.effectiveDesign): void {
  ordered("set_glass", { name, design });
}

/**
 * Цвет «Одного акцента» и базы «Своей палитры» — тоже начисто: поле HEX само
 * придерживает правки, пока цвет тянут пипеткой, и зовёт это не чаще раза в
 * сто двадцать миллисекунд.
 */
export function setAccentColor(color: string, design: Design = look.effectiveDesign): void {
  ordered("set_accent_color", { color, design });
}

export function setCustomColor(role: string, color: string, design: Design = look.effectiveDesign): void {
  ordered("set_custom_color", { role, color, design });
}

export function resetCustomColors(design: Design = look.effectiveDesign): void {
  ordered("reset_custom_colors", { design });
}

/** Единственный общий выбор дизайна; очередь сохраняет порядок быстрых нажатий. */
export function setDesign(design: Design): Promise<void> {
  const changed = queue.then(async () => {
    const reply = await invoke<Look>("set_design", { design });
    if (!reply || reply.design !== design || !reply.effectiveDesign) {
      throw new Error("Design command was not acknowledged");
    }
    adopt(reply);
  });
  // Очередь продолжает работать после отказа; вызывающий UI получает ошибку.
  queue = changed.catch(() => {});
  return changed;
}
