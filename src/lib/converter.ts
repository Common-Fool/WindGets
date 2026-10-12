import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";

/*
 * Конвертер: пачка файлов через ffmpeg. Работу ведёт Rust — очередь, проба
 * каждого файла, кодирование и прогресс; здесь только снимок и команды.
 */

export type Target = "mp4" | "gif" | "mp3" | "png" | "jpg";
export type Kind = "video" | "audio" | "image";
export type Status =
  | "probing"
  | "ready"
  | "unsupported"
  | "running"
  | "done"
  | "failed"
  | "cancelled";

export type Quality = "high" | "balanced" | "small";

export type Options = {
  target: Target | null;
  quality: Quality;
  /** Потолок видео по короткой стороне; 0 — как есть. */
  videoSize: number;
  bitrate: number;
  /** Длинная сторона GIF. */
  gifSize: number;
  gifFps: number;
  /** Длинная сторона картинки; 0 — как есть. */
  imageSize: number;
  from: string;
  to: string;
};

export type Item = {
  id: number;
  name: string;
  kind: Kind | null;
  duration: number | null;
  status: Status;
  progress: number | null;
  reason: string | null;
  output: string | null;
};

/**
 * Ряд потолков пачки: только те, что хоть что-то уменьшат (0 — «как есть»), и
 * тот, что пойдёт в работу. Выбранный, но отсеянный потолок Rust приводит к
 * «как есть» сам — ffmpeg с ним и так оставил бы размер исходника.
 */
export type SizeChoice = { offered: number[]; chosen: number };

export type ConverterView = {
  items: Item[];
  options: Options;
  targets: Target[];
  /** Во что превратится пачка: выбранное или, если оно не подходит, первое из годных. */
  target: Target | null;
  sizes: { video: SizeChoice; gif: SizeChoice; image: SizeChoice };
  mixed: boolean;
  trim: {
    enabled: boolean;
    duration: number | null;
    error: "badFrom" | "badTo" | "emptyRange" | null;
    /** Поля в секундах; пустое или нечитаемое — `null`. */
    from: number | null;
    to: number | null;
    /** Что обрезаем: у видео под ручкой кадр, у звука по полосе волна. */
    kind: Kind | null;
    /** Номер файла: сменился — волну и кадр просить заново. */
    subject: number | null;
  };
  running: boolean;
  overall: number | null;
  outDir: string | null;
  outDirCustom: boolean;
};

export const DEFAULT_OPTIONS: Options = {
  target: null,
  quality: "balanced",
  videoSize: 0,
  bitrate: 192,
  gifSize: 480,
  gifFps: 15,
  imageSize: 0,
  from: "",
  to: "",
};

const EMPTY: ConverterView = {
  items: [],
  options: DEFAULT_OPTIONS,
  targets: [],
  target: null,
  sizes: {
    video: { offered: [], chosen: 0 },
    gif: { offered: [], chosen: 0 },
    image: { offered: [], chosen: 0 },
  },
  mixed: false,
  trim: { enabled: false, duration: null, error: null, from: null, to: null, kind: null, subject: null },
  running: false,
  overall: null,
  outDir: null,
  outDirCustom: false,
};

export const useConverter = liveSnapshot<ConverterView>("converter_state", "converter", EMPTY);

export function pickFiles(): Promise<void> {
  return invoke("converter_pick");
}

export function removeFile(id: number): Promise<void> {
  return invoke("converter_remove", { id });
}

export function clearFiles(): Promise<void> {
  return invoke("converter_clear");
}

export function setOptions(options: Options): Promise<void> {
  return invoke("converter_set", { options });
}

export function startConverting(): Promise<void> {
  return invoke("converter_start");
}

export function cancelConverting(): Promise<void> {
  return invoke("converter_cancel");
}

export function revealConverted(id: number): Promise<void> {
  return invoke("converter_reveal", { id });
}

/** Где ручка в окне виджета, в точках webview: середина и её верх с низом. */
export type HandleBox = { x: number; top: number; bottom: number };

/**
 * Кадр видео в этом месте — своим окном над ручкой, поверх всех. Рисует его
 * Rust (`converter/popup.rs`): за границу своего окна разметка выйти не может.
 */
export function frameAt(at: number, handle: HandleBox): Promise<void> {
  return invoke("converter_frame", { at, handle });
}

export function hideFrame(): Promise<void> {
  return invoke("converter_frame_hide");
}

/** Волна звука: столбики от нуля до единицы, один раз на файл. */
export function waveform(): Promise<number[]> {
  return invoke("converter_wave");
}

/** Что заиграло: откуда и сколько секунд. */
export type Playing = { start: number; length: number };

/**
 * Послушать с места обрезки: от левой ручки вперёд (`tail` ложно) или
 * последние секунды перед правой. Отвечает, когда звук уже пошёл.
 */
export function listenFrom(from: number, until: number, tail: boolean): Promise<Playing> {
  return invoke("converter_listen", { from, until, tail });
}

export function hush(): Promise<void> {
  return invoke("converter_hush");
}

/**
 * Время для поля обрезки: `1:05`, `1:02:03`, а у коротких файлов с десятыми —
 * `0:07.5`. Читает его Rust (`parse_timecode`), и любая из этих форм ему
 * понятна.
 */
export function timecode(seconds: number, tenths: boolean): string {
  // Округляем до шага сразу, а не по частям: 5,96 — это 0:06, а не 0:05
  const steps = Math.round(Math.max(0, seconds) * (tenths ? 10 : 1));
  const whole = tenths ? Math.floor(steps / 10) : steps;
  const tenth = tenths ? steps % 10 : 0;
  const s = whole % 60;
  const m = Math.floor(whole / 60) % 60;
  const h = Math.floor(whole / 3600);

  const ss = String(s).padStart(2, "0") + (tenth > 0 ? `.${tenth}` : "");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}
