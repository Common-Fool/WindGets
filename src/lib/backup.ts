import { invoke } from "@tauri-apps/api/core";

/** Что лежит в копии — для подтверждения перед восстановлением. */
export type BackupSummary = {
  /** Имя выбранного файла, без папки. */
  file: string;
  /** Когда копию сняли, ISO 8601 в UTC. */
  created: string;
  /** Версия приложения, снявшего копию. */
  app: string;
  mail: number;
  totp: number;
  notes: number;
  todo: number;
  spotify: boolean;
  /** Сколько виджетов включено в копии. */
  widgets: number;
};

export type BackupErrorCode =
  | "busy"
  | "unreadable"
  | "secret"
  | "write"
  | "format"
  | "version"
  | "invalid"
  | "stage"
  | "nothing";

export type BackupError = { code: BackupErrorCode; file?: string };

export type BackupResult<T> = { ok: true; value: T } | { ok: false; error: BackupError };

async function call<T>(command: string): Promise<BackupResult<T>> {
  try {
    return { ok: true, value: await invoke<T>(command) };
  } catch (error: unknown) {
    console.warn(`${command} отказал`, error);
    const known = typeof error === "object" && error !== null && "code" in error;
    return { ok: false, error: known ? (error as BackupError) : { code: "write" } };
  }
}

/** Окно сохранения, затем файл. `null` — человек передумал; иначе путь записанного. */
export function exportBackup(): Promise<BackupResult<string | null>> {
  return call<string | null>("backup_export");
}

/** Окно открытия и разбор копии. `null` — человек передумал. */
export function inspectBackup(): Promise<BackupResult<BackupSummary | null>> {
  return call<BackupSummary | null>("backup_inspect");
}

/** Ставит разобранную копию на восстановление; приложение перезапустится само. */
export function restoreBackup(): Promise<BackupResult<null>> {
  return call<null>("backup_restore");
}
