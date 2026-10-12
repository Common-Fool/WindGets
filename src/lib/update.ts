import { invoke } from "@tauri-apps/api/core";
import { fileSize } from "./format";
import { liveSnapshot } from "./live";
import { openLink } from "./links";
import type { Text } from "./i18n";

/*
 * Обновления самого Windgets: раз в неделю — проверка выпуска на GitHub, а
 * скачивание и установка — только по нажатию (`src-tauri/src/update.rs`).
 */

export type UpdateFailure =
  | "noBuild"
  | "noChecksum"
  | "badChecksum"
  | "network"
  | "rateLimited"
  | "disk"
  | "cancelled"
  | "launch"
  | "dev";

export type UpdateJob =
  | { stage: "checking" }
  | { stage: "downloading"; received: number; total: number | null }
  | { stage: "installing" };

export type UpdateView = {
  current: string;
  /** Проверять раз в неделю самим. */
  auto: boolean;
  /** Секунды Unix. */
  checkedAt: number | null;
  available: { version: string; size: number } | null;
  job: UpdateJob | null;
  failure: UpdateFailure | null;
  /** Страница выпусков на GitHub. */
  page: string;
};

const EMPTY: UpdateView = {
  current: "",
  auto: true,
  checkedAt: null,
  available: null,
  job: null,
  failure: null,
  page: "",
};

export const useUpdate = liveSnapshot<UpdateView>("update_state", "update", EMPTY);

export function checkUpdate(): Promise<void> {
  return invoke("update_check");
}

export function installUpdate(): Promise<void> {
  return invoke("update_install");
}

export function cancelUpdate(): Promise<void> {
  return invoke("update_cancel");
}

export function setUpdateAuto(auto: boolean): Promise<void> {
  return invoke("update_set_auto", { auto });
}

export function openReleases(view: UpdateView): void {
  if (view.page !== "") openLink(view.page);
}

/** Ход работы одной строкой. */
export function updateJobText(t: Text, job: UpdateJob): string {
  switch (job.stage) {
    case "checking":
      return t("update.stage.checking");
    case "installing":
      return t("update.stage.installing");
    case "downloading":
      return job.total
        ? t("update.stage.downloadingOf", { received: fileSize(job.received), total: fileSize(job.total) })
        : t("update.stage.downloading", { received: fileSize(job.received) });
  }
}

/** Отказ: своё — у обновления, общее со скачиванием программ — оттуда. */
export function updateFailureText(t: Text, failure: UpdateFailure): string {
  switch (failure) {
    case "noBuild":
    case "launch":
    case "dev":
      return t(`update.failure.${failure}`);
    default:
      return t(`tools.failure.${failure}`);
  }
}

/** «Проверено 12 окт.» или «Ещё не проверялось». */
export function checkedText(t: Text, view: UpdateView): string {
  if (view.checkedAt === null) return t("update.never");
  const date = new Intl.DateTimeFormat(t.locale, { day: "numeric", month: "short" }).format(new Date(view.checkedAt * 1000));
  return t("update.checkedAt", { date });
}
