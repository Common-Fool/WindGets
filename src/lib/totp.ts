import { invoke } from "@tauri-apps/api/core";
import { liveSnapshot } from "./live";
import type { Key } from "./i18n";

/*
 * Коды 2FA: список записей и сами коды.
 *
 * ВАЖНО: **список и коды ходят порознь, и это главное отличие от остальных
 * виджетов.** Список меняется редко и приезжает событием, как у звука и почты;
 * коды меняются раз в период, и просит их карточка сама — ровно к границе,
 * которую Rust назвал в `validUntil`. Фонового цикла на той стороне нет вовсе.
 *
 * Секрета тут нет ни в одном типе, и это не совпадение: он не покидает Rust.
 */

/** Запись списка. Ни секрета, ни кода — только чем подписать строку. */
export type Row = {
  id: string;
  issuer: string;
  label: string;
  digits: number;
  period: number;
};

/**
 * Беда с файлом записей: `unreadable` — файл есть, но не прочитался, и правка
 * запрещена, пока он не прочитается; `setAside` — файл испорчен и отложен
 * рядом как `totp.bad.json`.
 */
export type Trouble = "unreadable" | "setAside";

export type View = {
  accounts: Row[];
  trouble: Trouble | null;
};

/** Заголовок и подсказка пустой карточки: при беде с файлом — о ней, а не «записей нет». */
export function emptyText(trouble: Trouble | null): { title: Key; hint: Key } {
  if (trouble === "unreadable") return { title: "totp.trouble.unreadable", hint: "totp.trouble.unreadableHint" };
  if (trouble === "setAside") return { title: "totp.trouble.setAside", hint: "totp.trouble.setAsideHint" };
  return { title: "totp.empty", hint: "totp.emptyHint" };
}

/** Код одной записи. `code` пуст, если секрет не расшифровался. */
export type Code = {
  id: string;
  code: string | null;
  validUntil: number;
};

export type Codes = {
  /** Мгновение, от которого считаны коды, в секундах эпохи. */
  now: number;
  codes: Code[];
  /** Часы заведомо неверны — они раньше самой новой записи. */
  clockSuspect: boolean;
};

const EMPTY: View = { accounts: [], trouble: null };

/**
 * Список записей. Один на окно, как у звука, и снимается, когда слушателей не
 * осталось (`live.ts`); там же порядок — сперва подписка, потом снимок:
 * запись, добавленная из настроек в промежутке, иначе не появилась бы на
 * карточке до следующей правки.
 */
export const useAccounts = liveSnapshot<View>("totp_state", "totp", EMPTY);

/**
 * Коды для перечисленных записей; пустой список — для всех.
 *
 * Спрятанное окно получает пустой ответ: это правило звука, и держит его Rust,
 * а не только мы.
 */
export function codesFor(ids: string[]): Promise<Codes> {
  return invoke<Codes>("totp_codes", { ids });
}

/** Завести запись: ссылкой `otpauth://` или голым секретом в base32. */
export function addAccount(input: string, issuer: string, label: string): Promise<Row> {
  return invoke<Row>("add_totp_account", { input, issuer, label });
}

export function renameAccount(id: string, issuer: string, label: string): Promise<void> {
  return invoke("rename_totp_account", { id, issuer, label });
}

/**
 * Порядок записей — он же порядок строк на карточке.
 *
 * Меняют его перетаскиванием в настройках, а не пунктом меню: «выше» и «ниже»
 * на десятке записей превращаются в два десятка нажатий.
 */
export function reorderAccounts(ids: string[]): Promise<void> {
  return invoke("reorder_totp_accounts", { ids });
}

export function removeAccount(id: string): Promise<void> {
  return invoke("remove_totp_account", { id });
}

/** Подпись строки: издатель и ярлык, но без пустых половин и лишней черты. */
export function titleOf(row: Row): string {
  if (row.issuer === "") return row.label;
  if (row.label === "") return row.issuer;
  return `${row.issuer} · ${row.label}`;
}

/** Код группами по три: «123 456» читается глазами вчетверо быстрее сплошного. */
export function grouped(code: string): string {
  if (code.length === 6) return `${code.slice(0, 3)} ${code.slice(3)}`;
  if (code.length === 8) return `${code.slice(0, 4)} ${code.slice(4)}`;
  return code;
}
