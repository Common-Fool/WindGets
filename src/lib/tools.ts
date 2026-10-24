import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";
import type { Text } from "./i18n";

/*
 * Докачиваемые программы конвертера, загрузчика и счётчика кадров: ffmpeg,
 * yt-dlp, QuickJS, PresentMon.
 *
 * В установщик они не кладутся — приложение скачивает их само, только по
 * нажатию, с официальных выпусков на GitHub, и само обновляет. Этот стор
 * показывает, что стоит, чего не хватает какому модулю и как идёт установка.
 */

export type ComponentId = "ffmpeg" | "ytDlp" | "quickJs" | "presentMon" | "headsetControl";
export type ModuleId = "converter" | "downloader" | "fps" | "battery";

export type ToolComponent = {
  id: ComponentId;
  name: string;
  /** Скачана приложением, взята из системы как есть или её нет. */
  presence: "missing" | "installed" | "system";
  version: string | null;
  downloadBytes: number;
  diskBytes: number;
  checkedAt: number | null;
};

export type FailureCode =
  | "noBuild"
  | "noChecksum"
  | "badChecksum"
  | "network"
  | "rateLimited"
  | "unpack"
  | "disk"
  | "cancelled"
  /** Удалить нельзя: программа сейчас работает. */
  | "inUse";

export type ToolsView = {
  components: ToolComponent[];
  /** `needs` — что модулю нужно вообще, `missing` — чего сейчас нет. */
  modules: { id: ModuleId; needs: ComponentId[]; missing: ComponentId[] }[];
  job: {
    component: ComponentId;
    stage: "checking" | "downloading" | "unpacking";
    received: number;
    total: number | null;
  } | null;
  failure: { component: ComponentId; code: FailureCode } | null;
  notice: { component: ComponentId; code: "upToDate" | "updated"; version: string } | null;
  /** Системный ffmpeg ещё ищется — «нет ffmpeg» пока не ответ. */
  probing: boolean;
};

const EMPTY: ToolsView = {
  components: [],
  modules: [],
  job: null,
  failure: null,
  notice: null,
  probing: true,
};

export const useTools = liveSnapshot<ToolsView>("tools_state", "tools", EMPTY);

/** Чего не хватает модулю — записями каталога, по порядку установки. */
export function missingFor(view: ToolsView, module: ModuleId): ToolComponent[] {
  const missing = view.modules.find((entry) => entry.id === module)?.missing ?? [];
  return missing
    .map((id) => view.components.find((component) => component.id === id))
    .filter((component): component is ToolComponent => component !== undefined);
}

/**
 * Всё, что нужно модулю, — для раздела настроек. Список приходит из Rust
 * (`Module::needs`): прежде здесь лежала его копия, и третий модуль разошёлся
 * бы с ней молча.
 */
export function componentsFor(view: ToolsView, module: ModuleId): ToolComponent[] {
  const needs = view.modules.find((entry) => entry.id === module)?.needs ?? [];
  return needs
    .map((id) => view.components.find((component) => component.id === id))
    .filter((component): component is ToolComponent => component !== undefined);
}

/** Модули, которым нужна эта программа, кроме названного. */
export function otherUsers(view: ToolsView, component: ComponentId, module: ModuleId): ModuleId[] {
  return view.modules
    .filter((entry) => entry.id !== module && entry.needs.includes(component))
    .map((entry) => entry.id);
}

export function nameOf(view: ToolsView, id: ComponentId): string {
  return view.components.find((component) => component.id === id)?.name ?? id;
}

export function installTools(module: ModuleId): Promise<void> {
  return invoke("tools_install", { module });
}

export function updateTool(component: ComponentId): Promise<void> {
  return invoke("tools_update", { component });
}

/** Удаляет скачанные программы. Системный ffmpeg Rust не тронет. */
export function removeTools(components: ComponentId[]): Promise<void> {
  return invoke("tools_remove", { components });
}

export function cancelTools(): Promise<void> {
  return invoke("tools_cancel");
}

/**
 * Причина отказа — одна на карточку и окно настроек. Прежде их было две, на
 * разных языках, и расходились они при первой же правке.
 */
export function failureText(t: Text, code: FailureCode): string {
  return t(`tools.failure.${code}`);
}

/** «FFmpeg», «yt-dlp и FFmpeg», «yt-dlp, QuickJS и FFmpeg» — союзом языка. */
export function listOf(t: Text, names: string[]): string {
  return new Intl.ListFormat(t.locale, { type: "conjunction" }).format(names);
}
