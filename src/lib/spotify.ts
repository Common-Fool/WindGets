import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type RepeatMode = "off" | "context" | "track";

export type Player = {
  track: string;
  artists: string;
  album: string;
  coverUrl: string | null;
  /** Трек в Spotify: ссылку обратно требует их политика разработчика. */
  link: string | null;
  isPlaying: boolean;
  progressMs: number;
  durationMs: number;
  shuffle: boolean;
  repeat: RepeatMode;
  /** null — устройство громкость через API не отдаёт. */
  volume: number | null;
  device: string | null;
  supportsVolume: boolean;
};

export type SpotifyView =
  | { status: "needsClientId" }
  | { status: "needsAuth"; clientId: string }
  | { status: "authorizing" }
  /** Никто не играет. `last` — последний виденный трек, если он известен. */
  | { status: "idle"; last: Player | null }
  | { status: "active"; player: Player }
  | { status: "error"; reason: string };

export type Playlist = {
  id: string;
  name: string;
  uri: string;
  coverUrl: string | null;
  tracks: number;
};

export type Place = "app" | "web";

/** Команда пульта. Форма повторяет размеченный union на стороне Rust. */
export type Action =
  | { kind: "play" }
  | { kind: "pause" }
  | { kind: "next" }
  | { kind: "previous" }
  | { kind: "shuffle"; on: boolean }
  | { kind: "repeat"; mode: RepeatMode }
  | { kind: "volume"; percent: number }
  | { kind: "seek"; positionMs: number }
  | { kind: "playContext"; uri: string };

/**
 * Состояние вместе с моментом получения: между опросами ползунок трека
 * двигает сам фронт, и без метки времени считать смещение не от чего.
 */
export type Snapshot = {
  view: SpotifyView;
  /** Момент прихода по часам окна, `performance`-независимо. */
  at: number;
};

type Listener = (snapshot: Snapshot) => void;

// Подписка одна на окно — та же схема, что у телеметрии, и так же снимается,
// когда слушателей не осталось: раздел Spotify в настройках иначе держал бы
// доставку каждого опроса в спрятанное окно до выхода
let current: Snapshot = { view: { status: "needsClientId" }, at: Date.now() };
const listeners = new Set<Listener>();
let unlisten: Promise<() => void> | null = null;

function publish(view: SpotifyView) {
  current = { view, at: Date.now() };
  for (const listener of listeners) listener(current);
}

function start() {
  if (unlisten !== null) return;

  invoke<SpotifyView>("spotify_view")
    .then(publish)
    .catch(() => {
      // До готовности бэкенда молчим — состояние придёт событием
    });

  unlisten = listen<SpotifyView>("spotify", (event) => publish(event.payload));
  unlisten.catch(() => {
    // Вне Tauri (обычный браузер) событий просто нет
  });
}

function stopIfIdle() {
  if (listeners.size > 0 || unlisten === null) return;
  const pending = unlisten;
  unlisten = null;
  pending.then((stop) => stop()).catch(() => {});
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    // Не сразу: `StrictMode` и смена раздела снимают и тут же заводят подписку
    setTimeout(stopIfIdle, 1000);
  };
}

function getSnapshot(): Snapshot {
  return current;
}

export function useSpotify(): Snapshot {
  return useSyncExternalStore(subscribe, getSnapshot);
}

/* ==================== Команды ==================== */

export function connect(clientId: string): Promise<void> {
  return invoke("spotify_connect", { clientId });
}

export function disconnect(): Promise<void> {
  return invoke("spotify_disconnect");
}

/**
 * Отказ команды пульта. `nowhere` — нажали «играть», а Спотифай нигде не
 * открыт: на это карточка отвечает слоем запуска, а не строкой отказа.
 */
export type ControlFailure = { kind: "nowhere" } | { kind: "failed"; reason: string };

export function control(action: Action): Promise<void> {
  return invoke("spotify_control", { action });
}

/**
 * Что пришло отказом `control`. Кроме отказа из Rust, `invoke` бросает и
 * своё — строкой, если сорвался сам вызов, — и оно становится обычной строкой
 * отказа.
 */
export function failureOf(reason: unknown): ControlFailure {
  if (typeof reason === "object" && reason !== null && "kind" in reason) {
    const failure = reason as ControlFailure;
    if (failure.kind === "nowhere" || failure.kind === "failed") return failure;
  }
  return { kind: "failed", reason: String(reason) };
}

export function fetchPlaylists(): Promise<Playlist[]> {
  return invoke("spotify_playlists");
}

export function openSpotify(place: Place): Promise<void> {
  return invoke("spotify_open", { place });
}

/* ==================== Мелочи отображения ==================== */

/** «3:56». Часы появляются только у длинных подкастов. */
export function formatTime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0:00";

  const total = Math.floor(ms / 1000);
  const seconds = total % 60;
  const minutes = Math.floor(total / 60) % 60;
  const hours = Math.floor(total / 3600);

  const mm = hours > 0 ? String(minutes).padStart(2, "0") : String(minutes);
  const ss = String(seconds).padStart(2, "0");

  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`;
}

/**
 * Позиция в треке на текущий момент.
 *
 * Опрос идёт раз в две секунды, и без досчёта ползунок дёргался бы рывками.
 * На паузе досчитывать нечего.
 */
export function positionNow(player: Player, receivedAt: number): number {
  if (!player.isPlaying) return player.progressMs;

  const elapsed = Date.now() - receivedAt;
  return Math.min(player.progressMs + Math.max(elapsed, 0), player.durationMs);
}

/** Следующий режим повтора по кругу — как в самом Спотифае. */
export function nextRepeat(mode: RepeatMode): RepeatMode {
  if (mode === "off") return "context";
  if (mode === "context") return "track";
  return "off";
}
