import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/**
 * Письмо в ящике.
 */
export type Letter = {
  /** Ключ ящика, из которого письмо: на вкладке «Все» ящики стоят вперемешку. */
  account: string;
  /** UID — уникален только внутри своего ящика, поэтому письмо — это пара. */
  uid: number;
  from: string;
  subject: string;
  /** Секунды эпохи Unix, UTC. */
  at: number;
  seen: boolean;
  preview: string | null;
  /** Адрес этого письма в браузере. */
  link: string | null;
  /** Адрес отправителя: имя стоит в `from`, а тут почта. */
  fromAddress: string | null;
  /** Адреса получателей, не больше пяти: списки рассылки бывают на сотню. */
  to: string[];
};

/** Вложение письма. Само содержимое наружу не отдаётся никогда. */
export type Attachment = {
  name: string;
  /** Размер в байтах, как лежит в письме. */
  size: number;
  /** Короткое слово типа: «pdf», «png», «zip», «file». */
  kind: string;
};

/** Что приезжает по нажатию на письмо. */
export type LetterBody = {
  text: string | null;
  attachments: Attachment[];
};

/**
 * Состояние входящих.
 */
export type Inbox = {
  unseen: number;
  letters: Letter[];
  /** Адрес веб-ящика в браузере. */
  mailbox: string | null;
};

/**
 * Что показывает один ящик.
 * Форма типа в точности повторяет разметку Rust View:
 * #[serde(tag = "status", rename_all = "camelCase")]
 */
export type BoxState =
  /** Ящик заведён не до конца — в сеть не ходят вовсе. */
  | { status: "noAccount" }
  /** Запись есть, писем ещё нет. */
  | { status: "loading" }
  /**
   * Снимок получен.
   * `stale` — ответа нет давно, на экране прошлый снимок.
   */
  | {
      status: "ready";
      inbox: Inbox;
      stale: boolean;
    }
  /**
   * Отказ. `reason` по-русски и предназначен для лога, а не для карточки;
   * карточке отвечает `kind`: «auth» — менять пароль, «network» — ждать.
   */
  | { status: "failed"; reason: string; kind: "auth" | "network" };

/** Ящик в снимке. */
export type MailBox = {
  id: string;
  /** Логин ящика: подсказка у точки вкладки и строка в дереве настроек. */
  address: string;
  view: BoxState;
};

/** Всё, что окна знают о почте, одним снимком. */
export type MailView = {
  /** Ящики в порядке заведения. */
  accounts: MailBox[];
  /** Строка содержимого под темой — вид карточки, одна на все ящики. */
  preview: boolean;
};

/** Сколько ящиков можно завести — то же число, что `MAX_ACCOUNTS` в Rust. */
export const MAX_ACCOUNTS = 5;

/** Письмо целиком: ящик и UID. Им письма различают отметки, кружки и окно. */
export function letterKey(letter: Letter): string {
  return `${letter.account}:${letter.uid}`;
}

/**
 * Ящик для окна настроек.
 */
export type MailAccount = {
  host: string;
  port: number;
  login: string;
  hasPassword: boolean;
  /** Своя ссылка на ящик. Пусто — значит «как выведется само». */
  mailbox: string;
  /** Что выведется само: подсказка серым в пустом поле настроек. */
  mailboxHint: string;
};

/**
 * Параметры ящика при сохранении.
 */
export type MailAccountConfig = {
  host: string;
  port: number;
  login: string;
  /** Пустая строка — вернуться к выведенному адресу. */
  mailbox: string;
};

type Listener = () => void;

/*
 * Подписка одна на окно, как у погоды и телеметрии. До первого ответа —
 * `null`: пустой список ящиков читался бы как «почты нет» и на миг показывал
 * кнопку настройки.
 */
let current: MailView | null = null;
const listeners = new Set<Listener>();
let started = false;

function publish(next: MailView) {
  current = next;
  for (const listener of listeners) listener();
}

function start() {
  if (started) return;
  started = true;
  // Ответ на первый вопрос мог прийти позже события: тогда он старше
  let heard = false;

  invoke<MailView>("mail_state")
    .then((first) => {
      if (!heard) publish(first);
    })
    .catch(() => {
      // До готовности бэкенда или вне Tauri молчим — состояние придёт событием
    });

  listen<MailView>("mail", (event) => {
    heard = true;
    publish(event.payload);
  }).catch(() => {
    // Вне Tauri (обычный браузер) события не приходят
  });
}

function subscribe(listener: Listener): () => void {
  start();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useMail(): MailView | null {
  return useSyncExternalStore(subscribe, () => current);
}

/**
 * Текст одного письма — для всплывающего окна.
 *
 * Отдельным походом и по нажатию: со списком тело не едет вовсе. Письма нынче
 * сплошь HTML без текстовой части, и в шестнадцать килобайт, которых хватает
 * списку на превью, видимый текст просто не попадает.
 */
export function getMailBody(letter: Letter): Promise<LetterBody> {
  return invoke<LetterBody>("mail_body", { account: letter.account, uid: letter.uid });
}

/**
 * Поиск по одному ящику.
 *
 * Считает его сервер, нам он почти ничего не стоит: уходит строка, приходит
 * список UID. Ответ живёт только в карточке — фоновый опрос он не трогает и
 * состояние виджета не подменяет.
 */
export function searchMail(account: string, query: string, unseen: boolean): Promise<Inbox> {
  return invoke<Inbox>("mail_search", { account, query, unseen });
}

/** Обновить все ящики сейчас, не дожидаясь шага опроса. */
export function refreshMail(): Promise<void> {
  return invoke<void>("mail_refresh").catch(() => {
    // Вне Tauri обновлять нечего
  });
}

/**
 * Реквизиты ящика (сервер, порт, логин, признак пароля). Пароль бэкенд не
 * возвращает никогда. Незнакомый ключ — пустая форма нового ящика.
 */
export function getMailAccount(id: string): Promise<MailAccount> {
  return invoke<MailAccount>("mail_account", { id });
}

/**
 * Сохранить ящик и получить его ключ. `id: null` — новый ящик.
 * `password: null` или пустая строка означает «оставить пароль прежним».
 */
export function setMailAccount(
  id: string | null,
  account: MailAccountConfig,
  password: string | null,
): Promise<string> {
  return invoke<string>("set_mail_account", { id, account, password });
}

/** Шаг проверки подключения — на нём она остановилась, если не прошла. */
export type CheckStage = "resolve" | "connect" | "tls" | "greeting" | "login" | "inbox";

/**
 * Чем кончилась проверка подключения. Повторяет `imap::Check` в Rust.
 *
 * `detail` — ответ системы или сервера как есть, без перевода: в нём коды, по
 * которым чинят, — `os error 11001`, `[AUTHENTICATIONFAILED]`.
 */
export type MailCheck =
  | { outcome: "passed"; messages: number; unseen: number; millis: number }
  | { outcome: "failed"; stage: CheckStage; timedOut: boolean; detail: string; millis: number };

/**
 * Проверить ящик тем, что стоит в форме: сервер, соединение, TLS, вход,
 * «Входящие». Пустой пароль — сохранённый у ящика `id`. До пятнадцати секунд.
 */
export function checkMail(id: string | null, account: MailAccountConfig, password: string | null): Promise<MailCheck> {
  return invoke<MailCheck>("mail_check", { id, account, password });
}

/**
 * Строка содержимого под темой письма — вид карточки, а не учётная запись.
 * Своей командой и сразу по нажатию: сохранение формы ящика её не трогает.
 */
export function setMailPreview(on: boolean): Promise<void> {
  return invoke<void>("set_mail_preview", { on });
}

/** Удалить ящик вместе с паролем. */
export function forgetMailAccount(id: string): Promise<void> {
  return invoke<void>("forget_mail_account", { id });
}

/** Пометить письма ящика прочитанными. */
export function markMailRead(account: string, uids: number[]): Promise<void> {
  return invoke<void>("mail_mark_read", { account, uids });
}

/** Пометить все письма ящика прочитанными. */
export function markAllMailRead(account: string): Promise<void> {
  return invoke<void>("mail_mark_all_read", { account });
}

/** Удалить письма ящика (перенести в корзину). */
export function deleteLetter(account: string, uids: number[]): Promise<void> {
  return invoke<void>("mail_delete", { account, uids });
}

/**
 * Письма пачкой по их ящикам: одно действие над отмеченными на вкладке «Все»
 * уходит по одной сессии в каждый ящик, а не по сессии на письмо.
 */
export function byAccount(letters: Letter[]): Map<string, number[]> {
  const groups = new Map<string, number[]>();
  for (const letter of letters) {
    const uids = groups.get(letter.account) ?? [];
    uids.push(letter.uid);
    groups.set(letter.account, uids);
  }
  return groups;
}
