import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Тип устройства: по нему плитка выбирает значок. */
export type Kind = "speakers" | "headphones" | "display" | "other";

export type Device = {
  id: string;
  /** Полное имя из системы: «Динамики (Redmi 电脑音箱)». */
  name: string;
  /** Подпись на плитке — обычно короткая, до первой скобки. */
  short: string;
  kind: Kind;
  current: boolean;
};

/**
 * Какая панель раскрыта в карточке.
 *
 * Едет в Rust: списки он собирает только под раскрытую панель, а свёрнутая
 * стоит ему двух вызовов вместо полутора десятков. Раскрыта всегда одна —
 * место в карточке одно.
 */
export type Panel = "none" | "outputs" | "inputs" | "mixer";

/** Строка микшера: одна звучащая программа. */
export type Track = {
  /** Имя процесса без расширения — оно же ключ строки и адрес команды. */
  id: string;
  name: string;
  volume: number;
  muted: boolean;
};

/** Значок программы: точки RGBA, готовые лечь в холст. */
export type Icon = { width: number; height: number; pixels: string };

export type SoundState = {
  /** `null` — устройства вывода нет вовсе. Не то же самое, что нулевая громкость. */
  output: { volume: number; muted: boolean } | null;
  microphone: { muted: boolean } | null;
  /** Нынешний выход: имя и значок во всех раскладках. */
  current: Device | null;
  /** Устройства вывода. Пусто, пока их панель свёрнута. */
  devices: Device[];
  /** Устройства ввода. Пусто, пока их панель свёрнута. */
  inputs: Device[];
  /** Кто сейчас звучит. Пусто, пока микшер свёрнут. */
  sessions: Track[];
  /** Переключать вывод нечем: `IPolicyConfig` не завёлся. */
  canSwitch: boolean;
  /**
   * Громкость, которую вернёт значок, если сейчас ноль.
   *
   * Приходит из Rust, потому что предсказать её тут нечем: до первого
   * ненулевого показания взять её неоткуда, и своё число разошлось бы с тем,
   * что поставит система.
   */
  restore: number;
};

export type Action =
  | { kind: "volume"; percent: number }
  | { kind: "toggleMute" }
  | { kind: "toggleMicrophone" }
  | { kind: "nextDevice" }
  | { kind: "nextInput" }
  | { kind: "device"; id: string }
  | { kind: "input"; id: string }
  | { kind: "sessionVolume"; id: string; percent: number }
  | { kind: "sessionMute"; id: string; muted: boolean }
  | { kind: "panel"; panel: Panel };

const EMPTY: SoundState = {
  output: null,
  microphone: null,
  current: null,
  devices: [],
  inputs: [],
  sessions: [],
  canSwitch: false,
  restore: 0,
};

/*
 * Значки программ, спрошенные по одному разу.
 *
 * Держим у окна, а не возим в каждом событии: точки весят вчетверо больше всей
 * остальной строки, а меняться значку не с чего. `null` в карте — «спрашивали,
 * значка нет»: без него мы просили бы его снова каждую перерисовку.
 */
const icons = new Map<string, Icon | null>();
const asking = new Set<string>();

/** Значок программы, если он уже спрошен. */
export function iconOf(name: string): Icon | null | undefined {
  return icons.get(name);
}

/**
 * Спрашивает значок и будит подписчиков, когда он придёт.
 *
 * Один заход на программу: повторов не делаем даже при отказе — значок либо
 * есть, либо его не дадут и во второй раз.
 */
export function askIcon(name: string): void {
  if (icons.has(name) || asking.has(name)) return;
  asking.add(name);

  invoke<Icon | null>("sound_icon", { name })
    .then((icon) => {
      icons.set(name, icon);
      // Своя копия состояния: подписчики сравнивают по ссылке, и то же самое
      // значение они бы просто не заметили
      publish({ ...current });
    })
    .catch(() => {
      icons.set(name, null);
    })
    .finally(() => {
      asking.delete(name);
    });
}

type Listener = (state: SoundState) => void;

/*
 * Подписка одна на окно, как у телеметрии и пинга: событие приходит дважды в
 * секунду, и заводить по listen на каждый вызов хука незачем.
 */
let current: SoundState = EMPTY;
const listeners = new Set<Listener>();
let started = false;

function publish(next: SoundState) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  // Спрашиваем сразу: окно, открытое между тиками, иначе ждало бы полсекунды.
  // Но сперва подписка, потом вопрос (как в `live.ts`): событие, ушедшее до
  // слушателя, пропало бы, а одинаковых снимков Rust повторно не шлёт — окно
  // стояло бы на старом до следующей перемены
  let heard = false;
  listen<SoundState>("sound", (event) => {
    heard = true;
    publish(event.payload);
  })
    .then(() => invoke<SoundState>("sound_state"))
    .then((initial) => {
      if (!heard) publish(initial);
    })
    .catch(() => {
      // Вне Tauri (обычный браузер) звука нет; до готовности бэкенда молчим —
      // состояние придёт событием
    });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useSound(): SoundState {
  return useSyncExternalStore(subscribe, () => current);
}

/**
 * Отправляет команду и не ждёт результата.
 *
 * Ответ тут не нужен вовсе: настоящее подтверждение — следующий тик опроса,
 * который принесёт новое состояние. Отказ логируем в консоль и живём дальше.
 */
export function control(action: Action): void {
  void invoke("sound_control", { action }).catch((error: unknown) => {
    console.warn("звук не послушался", error);
  });
}

/** Тихо ли сейчас. Ноль громкости — такая же тишина, как и молчание. */
export function silent(output: SoundState["output"]): boolean {
  return output === null || output.muted || output.volume === 0;
}
