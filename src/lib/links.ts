import { invoke } from "@tauri-apps/api/core";

/*
 * Ссылки внутри текста, который написал человек: заметки и дела.
 *
 * Открывает их Rust отдельным процессом. Из фронта тут не уйти никуда: переход
 * по `href` увёл бы само окно виджета на страницу, а `window.open` в webview
 * без своего браузера бесполезен.
 */

/**
 * Что считаем ссылкой.
 *
 * Только `http` и `https`, и только до первого пробела: `www.` без схемы и
 * голые домены сюда не входят намеренно — «сайт.рф» в тексте дела ссылкой не
 * задумывался, а угадывать за человека значит однажды угадать неверно.
 */
const LINK = /https?:\/\/[^\s<>"']+/g;

/**
 * Ссылка со своим словом: `[unsubscribe](https://…)`.
 *
 * Так приходит письмо: в HTML адрес спрятан в свойстве тега, а видно только
 * слово, и при превращении в текст адрес пропадал бесследно. Rust выносит его
 * в эту запись, а карточка показывает слово и открывает адрес — то есть ровно
 * то, что было в письме.
 *
 * Запись выбрана та же, что у Markdown: если разбор однажды не сработает,
 * человек увидит понятную строку, а не мусор из служебных знаков.
 */
const LABELLED = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/;

/** Обе записи разом, в порядке появления в тексте. */
const ANY_LINK = new RegExp(`${LABELLED.source}|${LINK.source}`, "g");

/**
 * Знаки, которые не входят в ссылку, даже если стоят вплотную.
 *
 * Ссылка в конце предложения кончается точкой, а взятая в скобки — скобкой;
 * забрать их в адрес значит открыть его битым.
 */
const TAIL = /[.,;:!?)\]}»"']+$/;

/**
 * Закрывающая скобка, у которой в адресе есть пара, — часть адреса, а не
 * предложения: `…/wiki/Rust_(programming_language)` без неё открывался бы
 * обрезанным, «страница не найдена».
 */
function withPairedParens(whole: string, url: string): string {
  let kept = url;
  while (kept.length < whole.length && whole[kept.length] === ")") {
    const opened = kept.split("(").length - 1;
    const closed = kept.split(")").length - 1;
    if (opened <= closed) break;
    kept += ")";
  }
  return kept;
}

/** Кусок текста: обычный или ссылка. */
export type Piece = {
  /** Что видно человеку. */
  text: string;
  link: boolean;
  /**
   * Куда ведёт. У голой ссылки совпадает с текстом, у ссылки со своим словом —
   * нет: там видно слово, а открывается адрес.
   */
  href: string;
};

/**
 * Режет текст на куски: обычные и ссылки.
 *
 * Пустые куски не отдаём — из них потом получались бы пустые узлы разметки.
 */
export function pieces(text: string, verbatim = false): Piece[] {
  const out: Piece[] = [];
  let at = 0;

  for (const found of text.matchAll(ANY_LINK)) {
    const start = found.index;
    const [whole, label, target] = found;

    if (start > at) {
      out.push({ text: text.slice(at, start), link: false, href: "" });
    }

    if (label !== undefined && target !== undefined) {
      // Ссылка со своим словом: видно слово, открывается адрес. Дословно —
      // вся запись целиком: подложке заметки нельзя показывать меньше знаков,
      // чем стоит в поле над ней
      out.push({ text: verbatim ? whole : label, link: true, href: target });
      at = start + whole.length;
      continue;
    }

    // Голая ссылка: хвостовые знаки предложения в адрес не входят
    const url = withPairedParens(whole, whole.replace(TAIL, ""));
    out.push({ text: url, link: true, href: url });
    at = start + url.length;
  }

  if (at < text.length) out.push({ text: text.slice(at), link: false, href: "" });
  return out;
}

/**
 * Открывает ссылку в браузере.
 *
 * Проверку схемы делает Rust, а не мы: строка приходит из текста, и решение о
 * запуске должно приниматься там же, где запуск.
 */
export function openLink(url: string): void {
  void invoke("open_link", { url }).catch(() => {
    // Вне Tauri звать некого
  });
}
