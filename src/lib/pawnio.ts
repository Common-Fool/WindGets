import { invoke } from "@tauri-apps/api/core";
import { fileSize } from "./format";
import { liveSnapshot } from "./live";
import { openLink } from "./links";
import type { Text } from "./i18n";

/*
 * Драйвер PawnIO: через него LibreHardwareMonitor читает процессор и чип
 * платы. Ставится только по нажатию — официальным установщиком автора с
 * GitHub, со сверкой SHA-256 (`src-tauri/src/pawnio.rs`).
 */

export type DriverStatus =
  /** Ответа Rust ещё не было: карточку не запираем, иначе она мигала бы. */
  | "unknown"
  | "ready"
  | "missing"
  | "outdated"
  /** Стоит, но не загружен — нужна перезагрузка. */
  | "restart";

export type DriverFailure =
  | "noBuild"
  | "noChecksum"
  | "badChecksum"
  | "network"
  | "rateLimited"
  | "disk"
  | "cancelled"
  | "installer"
  | "uninstaller";

export type DriverJob =
  | { stage: "checking" }
  | { stage: "downloading"; received: number; total: number | null }
  | { stage: "installing" }
  | { stage: "removing" };

export type DriverView = {
  status: DriverStatus;
  version: string | null;
  /** Поставлен Windgets — только такой предлагаем удалить. */
  ours: boolean;
  job: DriverJob | null;
  failure: { code: DriverFailure; exit: number | null } | null;
  downloadBytes: number;
  /** Выпуски автора на GitHub. */
  page: string;
};

const EMPTY: DriverView = {
  status: "unknown",
  version: null,
  ours: false,
  job: null,
  failure: null,
  downloadBytes: 0,
  page: "",
};

export const usePawnio = liveSnapshot<DriverView>("pawnio_state", "pawnio", EMPTY);

/** Карточка процессора заперта: драйвера нет, он устарел или ждёт перезагрузки. */
export function needsDriver(view: DriverView): boolean {
  return view.status !== "unknown" && view.status !== "ready";
}

export function installPawnio(): Promise<void> {
  return invoke("pawnio_install");
}

export function removePawnio(): Promise<void> {
  return invoke("pawnio_remove");
}

export function cancelPawnio(): Promise<void> {
  return invoke("pawnio_cancel");
}

export function openPawnioPage(view: DriverView): void {
  if (view.page !== "") openLink(view.page);
}

/** Ход работы одной строкой — в карточке и в настройках. */
export function jobText(t: Text, job: DriverJob): string {
  switch (job.stage) {
    case "checking":
      return t("pawnio.stage.checking");
    case "installing":
      return t("pawnio.stage.installing");
    case "removing":
      return t("pawnio.stage.removing");
    case "downloading":
      return job.total
        ? t("pawnio.stage.downloadingOf", { received: fileSize(job.received), total: fileSize(job.total) })
        : t("pawnio.stage.downloading", { received: fileSize(job.received) });
  }
}

/** Отказ: скачивание говорит словами программ, установщик — своим кодом. */
export function failureText(t: Text, failure: NonNullable<DriverView["failure"]>): string {
  const exit = failure.exit === null ? "?" : String(failure.exit);
  switch (failure.code) {
    case "installer":
      return t("pawnio.failure.installer", { exit });
    case "uninstaller":
      return t("pawnio.failure.uninstaller", { exit });
    default:
      return t(`tools.failure.${failure.code}`);
  }
}

/** Состояние в настройках: «работает · версия 2.2.0.0», «не установлен». */
export function statusText(t: Text, view: DriverView): string {
  switch (view.status) {
    case "ready":
      return view.version === null ? t("pawnio.status.readyUnknown") : t("pawnio.status.ready", { version: view.version });
    case "outdated":
      return t("pawnio.status.outdated", { version: view.version ?? "?" });
    case "restart":
      return t("pawnio.status.restart");
    case "missing":
    case "unknown":
      return t("pawnio.status.missing");
  }
}
