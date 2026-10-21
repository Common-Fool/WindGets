import { invoke } from "@tauri-apps/api/core";

// Снимок уходит сразу, даже если прежняя запись ещё ждёт диск: очередь
// в webview погибла бы вместе с выключенным окном. Rust сравнит эти номера.
let session = 0;
let revision = 0;

/*
 * Заметки: несколько страниц текста в каталоге настроек.
 *
 * Подписки тут нет и не будет — состояние никем, кроме самого окна, не
 * меняется, и слушать некого. Модуль держит `invoke` подальше от карточки, как
 * у всех прочих виджетов.
 */

/** Книга заметок: страницы и та, что открыта. */
export type Book = {
  pages: string[];
  /** Открытая страница. Живёт на диске: с ней и открывается окно. */
  active: number;
};

/** Что лежит на диске. Заметок ещё не вели — ни одной страницы. */
export function loadNotes(): Promise<Book> {
  return invoke<Book & { session: number }>("notes_pages").then(({ session: nextSession, ...book }) => {
    if (nextSession > session) {
      session = nextSession;
      revision = 0;
    }
    return book;
  });
}

/**
 * Кладёт книгу на диск.
 *
 * Отказ не глотаем: карточка обязана сказать, что написанное не сохранилось.
 * Причину при этом показывать нечем — она русская и уходит в лог, а в карточке
 * весь текст английский.
 */
export function saveNotes(book: Book): Promise<void> {
  return invoke("set_notes_pages", { book, session, revision: ++revision });
}
