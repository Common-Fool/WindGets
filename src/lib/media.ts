import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/*
 * Медиасессии Windows: активные плееры, пульт управления и список источников.
 *
 * Rust опрашивает системные медиасессии раз в секунду, пока окно видно, а
 * снимок рассылает только когда он поменялся — на играющем треке это смена
 * трека, паузы или кнопок, а не каждый такт. Поэтому полоса между снимками едет
 * за счёт досчёта позиции от `measuredAt` здесь (`playedAt`).
 *
 * Тяжёлые данные (обложки треков) запрашиваются отдельно через artOf только при
 * смене ключа art, чтобы не гонять base64 в каждом снимке состояния.
 */

/** Источник: приложение, которое хоть раз играло. Для окна настроек. */
export type Source = {
  id: string;
  title: string;
  enabled: boolean;
  seenAt: number;
};

/** Одна медиасессия Windows. */
export type Track = {
  /** Ключ источника (`SourceAppUserModelId`). */
  source: string;
  /** Имя источника для показа. */
  title: string;
  song: string;
  artist: string;
  playing: boolean;
  canPlay: boolean;
  canNext: boolean;
  canPrevious: boolean;
  canSeek: boolean;
  /** Позиция в секундах на момент `measuredAt`. */
  position: number;
  /** Длина в секундах; 0 значит «неизвестна». */
  duration: number;
  /** Когда снят замер, в миллисекундах эпохи. */
  measuredAt: number;
  /** Ключ обложки: меняется со сменой трека, `null` — обложки нет. */
  art: string | null;
};

export type View = {
  /** Только разрешённые источники, порядок от Windows. */
  sessions: Track[];
  /** Ключ источника выбранной сессии или `null` — «сама выберет». */
  current: string | null;
  /** Всё, что видели, включая выключенное. Для окна настроек. */
  sources: Source[];
};

const EMPTY: View = { sessions: [], current: null, sources: [] };

type Listener = (state: View) => void;

let current: View = EMPTY;
const listeners = new Set<Listener>();
let started = false;

function publish(next: View) {
  current = next;
  for (const listener of listeners) listener(next);
}

function start() {
  if (started) return;
  started = true;

  let heard = false;

  // Спрашиваем и подписываемся разом, и порядок ответов не гарантирован:
  // запоздавший снимок затёр бы список, только что пришедший событием
  invoke<View>("media_state")
    .then((initial) => {
      if (heard) return;
      publish(initial);
    })
    .catch(() => {
      // Вне Tauri сессий просто нет
    });

  listen<View>("media", (event) => {
    heard = true;
    publish(event.payload);
  }).catch(() => {});
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useMedia(): View {
  return useSyncExternalStore(subscribe, () => current);
}

/** Кнопки пульта. */
export function control(
  source: string,
  action: "play" | "pause" | "next" | "previous",
): Promise<void> {
  return invoke("media_control", { source, action });
}

/** Перемотка, в секундах от начала. */
export function seek(source: string, position: number): Promise<void> {
  return invoke("media_seek", { source, position });
}

/** Какую сессию показывать, когда играют двое разом. */
export function pickSession(source: string): Promise<void> {
  return invoke("pick_media_session", { source });
}

/** Показывать ли источник. Настройки. */
export function setSource(id: string, enabled: boolean): Promise<void> {
  return invoke("set_media_source", { id, enabled });
}

/**
 * Обложка отдельной командой, а не в каждом снимке: это байты в base64, и
 * гонять их в окно раз в секунду незачем. Спрашивают по смене ключа `art`.
 */
export function artOf(source: string): Promise<string | null> {
  return invoke<string | null>("media_art", { source });
}

/**
 * Где сейчас игла, в секундах.
 *
 * Rust присылает позицию раз в секунду, а полоса обязана ехать плавно —
 * поэтому между снимками позицию досчитываем сами от `measuredAt`. На паузе
 * ничего не досчитываем: там позиция и есть позиция.
 *
 * `now` — `Date.now()`. Ответ зажат в [0, duration], а при неизвестной длине
 * (duration <= 0) — снизу нулём.
 */
export function playedAt(track: Track, now: number): number {
  const elapsed = track.playing ? Math.max(0, (now - track.measuredAt) / 1000) : 0;
  const position = track.position + elapsed;
  if (track.duration > 0) {
    return Math.min(Math.max(0, position), track.duration);
  }
  return Math.max(0, position);
}

/**
 * Какую сессию показывать.
 *
 * Выбранную руками, если она ещё в списке; иначе первую играющую; иначе
 * первую вообще; на пустом списке `null`.
 */
export function shown(view: View): Track | null {
  if (view.sessions.length === 0) return null;

  if (view.current !== null) {
    const picked = view.sessions.find((session) => session.source === view.current);
    if (picked) return picked;
  }

  const playing = view.sessions.find((session) => session.playing);
  if (playing) return playing;

  return view.sessions[0] ?? null;
}

/**
 * Время под полосой: «3:07», «1:02:33». Отрицательное и нечисловое — «0:00».
 * Часы появляются только когда они есть.
 */
export function clock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return "0:00";

  const total = Math.floor(seconds);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);

  const ss = String(s).padStart(2, "0");
  if (h > 0) {
    const mm = String(m).padStart(2, "0");
    return `${h}:${mm}:${ss}`;
  }
  return `${m}:${ss}`;
}

/** Подпись строки: исполнитель и название без пустых половин и лишней черты. */
export function titleOf(track: Track): string {
  if (track.artist === "") return track.song;
  if (track.song === "") return track.artist;
  return `${track.artist} · ${track.song}`;
}
