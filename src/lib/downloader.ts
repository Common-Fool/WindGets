import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";
import type { Text } from "./i18n";

/*
 * Загрузчик: видео или звук по ссылке через yt-dlp. Качает, склеивает и
 * переносит готовое Rust; здесь снимок и команды.
 */

export type Mode = "video" | "audio";
export type Quality = "p720" | "p1080" | "best";

export type Status =
  | "idle"
  | "preparing"
  | "updating"
  | "downloading"
  | "processing"
  | "saving"
  | "done"
  | "failed"
  | "cancelled";

export type Reason = "broken" | "signIn" | "unavailable" | "unsupported" | "network" | "failed";

/** Скачанное за этот запуск: только в памяти Rust, до выхода. */
export type Saved = {
  id: number;
  url: string;
  title: string;
  /** Имя файла в папке вывода. */
  file: string;
  mode: Mode;
};

export type DownloaderView = {
  status: Status;
  url: string | null;
  title: string | null;
  part: { index: number; count: number } | null;
  percent: number | null;
  downloaded: number | null;
  total: number | null;
  /** Байт в секунду. */
  speed: number | null;
  eta: number | null;
  step: "merging" | "converting" | null;
  reason: Reason | null;
  detail: string | null;
  output: string | null;
  mode: Mode;
  quality: Quality;
  outDir: string | null;
  outDirCustom: boolean;
  /** Свежее сверху. */
  history: Saved[];
};

const EMPTY: DownloaderView = {
  status: "idle",
  url: null,
  title: null,
  part: null,
  percent: null,
  downloaded: null,
  total: null,
  speed: null,
  eta: null,
  step: null,
  reason: null,
  detail: null,
  output: null,
  mode: "video",
  quality: "p1080",
  outDir: null,
  outDirCustom: false,
  history: [],
};

export const useDownloader = liveSnapshot<DownloaderView>("downloader_state", "downloader", EMPTY);

export function startDownload(url: string, mode: Mode, quality: Quality): Promise<void> {
  return invoke("downloader_start", { url, mode, quality });
}

export function setDownloadOptions(mode: Mode, quality: Quality): Promise<void> {
  return invoke("downloader_options", { mode, quality });
}

export function cancelDownload(): Promise<void> {
  return invoke("downloader_cancel");
}

/** Показывает файл в папке: без номера — последний скачанный. */
export function revealDownload(id?: number): Promise<void> {
  return invoke("downloader_reveal", { id: id ?? null });
}

/** Убирает строку истории, без номера — всю историю. Файлы остаются. */
export function forgetDownload(id?: number): Promise<void> {
  return invoke("downloader_forget", { id: id ?? null });
}

/** Идёт работа — кнопка «скачать» становится «отменить». */
export function isBusy(status: Status): boolean {
  return ["preparing", "updating", "downloading", "processing", "saving"].includes(status);
}

/** Причина отказа для карточки — коротко, одной строкой. */
export function reasonText(t: Text, reason: Reason, detail: string | null): string {
  if (reason === "failed") return detail ?? t("downloader.reason.failed");
  return t(`downloader.reason.${reason}`);
}
