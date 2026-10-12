import { useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KEEPS_THE_MOUSE } from "./drag";

/*
 * Жесты по карточке: меню по правой кнопке и нажатие по левой.
 *
 * Оба спрашивают одно и то же — попали в пустое место карточки или в орган
 * управления, — и спрашивают тем же списком, которым это решает
 * перетаскивание. Правило «где у карточки пустота» в проекте одно, и
 * разойтись трём его копиям было бы вопросом времени.
 */

/**
 * Поле, в котором правят текст.
 *
 * ВАЖНО: **у него своё меню, и отбирать его нельзя.** Меню WebView2 мы гасим
 * везде — но в поле ввода это меню и есть «Вырезать / Копировать / Вставить»,
 * без которого в поле города и в поле Client ID нечем вставить строку мышью.
 * Поймано independent-ревью, а не глазами: своё поле правят с клавиатуры, и
 * пропажу пункта «Вставить» замечаешь только когда полез мышью.
 */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest(EDITABLE) !== null;
}

/**
 * Поля, в которых правят текст.
 *
 * ВАЖНО: **не всякий `input` — поле.** Ползунок, галочка и цвет — тоже
 * `input`, но вставлять в них нечего, и правый щелчок по ползунку громкости
 * отдавал окно меню WebView2 целиком: «Обновить», «Сохранить как», «Печать».
 */
const EDITABLE =
  'textarea, [contenteditable], input:not([type="range"]):not([type="checkbox"]):not([type="radio"]):not([type="color"]):not([type="button"])';

/**
 * Карточка, у содержимого которой своего правого щелчка нет вовсе: меню
 * виджета открывается по ней везде, хоть по кнопке, хоть по ползунку. Звук
 * так и живёт — списки у него раскрывают явные кнопки, а без этого меню
 * ловилось бы только у края и в редких просветах между органами.
 */
const MENU_ANYWHERE = "[data-menu-anywhere]";

/** Пустое место: всё, кроме кнопок, полей, ползунков и выделяемого текста. */
function isEmptySpace(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  // Через `closest`, потому что нажатие приходит на самый глубокий элемент —
  // на подпись внутри кнопки, а не на саму кнопку
  return target.closest(KEEPS_THE_MOUSE) === null;
}

/**
 * Насколько близко к краю окна должна быть мышь, чтобы открылось меню виджета.
 *
 * ВАЖНО: **шесть точек от края съедает полоса захвата.** `snap.rs` отвечает на
 * `WM_NCHITTEST` рамкой на шести крайних точках, и нажатие там уходит в
 * неклиентскую часть окна — до webview оно не доходит вовсе. Поэтому полоса
 * меню шире на эти шесть: работающими остаются примерно десять точек, что и
 * задумано.
 */
const MENU_EDGE = 16;

/** Мышь у края окна — там, где карточка не занята содержимым. */
function nearEdge(event: MouseEvent): boolean {
  const width = document.documentElement.clientWidth;
  const height = document.documentElement.clientHeight;

  return (
    event.clientX <= MENU_EDGE ||
    event.clientY <= MENU_EDGE ||
    width - event.clientX <= MENU_EDGE ||
    height - event.clientY <= MENU_EDGE
  );
}

/**
 * Меню виджета по правой кнопке у края окна — и запрет меню WebView2 во всём
 * окне.
 *
 * Ставится один раз на окно, в оболочке (`main.tsx`), а не в каждом виджете:
 * запрет чужого меню нужен и настройкам, и витрине, и отладке, а забытый в
 * новом виджете вызов вернул бы туда «Обновить / Сохранить как / Печать».
 *
 * Открывается оно **на пустом месте карточки или у края окна** — двух условий,
 * а не одного. Пустое место есть не у всех: список дел занят строками и полем
 * ввода, и меню в нём было не открыть вовсе; край же есть у любого окна, всегда
 * на одном месте, и от содержимого не зависит. Вместе они закрывают и тот
 * случай, и другой.
 *
 * Заметка при этом пустым местом становится сама: пока правку не разрешили
 * двойным нажатием, поле мышь не ловит, и меню открывается по всей карточке.
 * Разрешили — поле берёт правую кнопку себе, и это уже его «Вырезать /
 * Копировать / Вставить».
 *
 * Правая кнопка по содержимому остаётся содержимому: строка дела открывает
 * своё меню. Карточка, у которой своего правого щелчка нет, помечается
 * `data-menu-anywhere`, и меню виджета открывается по ней везде (звук). Меню
 * движка гасим всё равно — оно не нужно нигде, кроме полей ввода.
 */
export function useWidgetMenu(): void {
  useEffect(() => {
    const open = (event: MouseEvent) => {
      // Полю ввода оставляем его собственное меню целиком
      if (isEditable(event.target)) return;

      event.preventDefault();
      const anywhere = event.target instanceof Element && event.target.closest(MENU_ANYWHERE) !== null;
      if (!anywhere && !isEmptySpace(event.target) && !nearEdge(event)) return;

      // Координаты от левого верхнего угла окна: их же ждёт `popup_at`
      void invoke("open_widget_menu", { x: event.clientX, y: event.clientY }).catch(() => {
        // Вне Tauri звать некого — в браузере остаётся просто погашенное меню
      });
    };

    document.addEventListener("contextmenu", open);
    return () => document.removeEventListener("contextmenu", open);
  }, []);
}

/**
 * Насколько мышь может уехать, чтобы нажатие всё ещё считалось нажатием.
 *
 * То же число, что порог перетаскивания в `drag.ts`, и по той же причине:
 * щелчок почти всегда идёт с крошечным сдвигом.
 */
const CLICK_SLACK = 4;

/**
 * Нажатие левой кнопкой по пустому месту карточки.
 *
 * Обработчику достаётся то, по чему нажали. Нужно это карточкам, у которых
 * пустое место не одно на всю карточку: у погоды нажатие по основному блоку и
 * по клетке дня раскрывают разное. Делать клетки кнопками нельзя — за кнопку
 * окно не потащишь, а тянут виджеты за что угодно, кроме органов управления.
 *
 * Элемент берётся от нажатия, а не от отпускания: нажатие, начатое на одной
 * клетке и отпущенное на соседней, — это промах мышью, и отвечать на него
 * второй клеткой было бы неожиданностью.
 *
 * ВАЖНО: **начатое перетаскивание нажатием быть не должно.** Обычно это
 * получается само — перетаскивание уводит мышь в модальный цикл Windows, и
 * `click` во фронт не приходит вовсе, — но полагаться на это нельзя: порог
 * сдвига мы проходим не всегда. Поэтому меряем сами и молча выбрасываем
 * нажатие, у которого мышь ушла дальше порога.
 */
export function useCardClick(handler: (target: Element | null) => void): void {
  const act = useRef(handler);

  useEffect(() => {
    act.current = handler;
  }, [handler]);

  useEffect(() => {
    let from: { x: number; y: number; target: Element | null } | null = null;

    const press = (event: MouseEvent) => {
      from =
        event.button === 0 && isEmptySpace(event.target)
          ? {
              x: event.screenX,
              y: event.screenY,
              target: event.target instanceof Element ? event.target : null,
            }
          : null;
    };

    const release = (event: MouseEvent) => {
      const start = from;
      from = null;
      if (start === null) return;

      const far =
        Math.abs(event.screenX - start.x) >= CLICK_SLACK ||
        Math.abs(event.screenY - start.y) >= CLICK_SLACK;
      if (far) return;

      act.current(start.target);
    };

    document.addEventListener("mousedown", press);
    document.addEventListener("click", release);
    return () => {
      document.removeEventListener("mousedown", press);
      document.removeEventListener("click", release);
    };
  }, []);
}
